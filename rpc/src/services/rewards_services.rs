use std::{collections::BTreeMap, str::FromStr};

use crate::{
    helpers::{parse_block_hash, BlockMetadata, RpcServiceError, MAIN_CHAIN_ID, parse_chain_id, BlockOperations},
    parse_block_hash_or_fail, RpcServiceEnvironment,
};
use anyhow::Error;
use crypto::hash::{ChainId, ProtocolHash, BlockHash};
use num::{BigInt, bigint::Sign, Signed, BigRational};
use serde::{Deserialize, Serialize};
use storage::{BlockStorageReader, BlockJsonData, BlockHeaderWithHash, OperationsStorage, OperationsStorageReader, BlockAdditionalData};
use storage::{
    cycle_eras_storage::CycleEra, BlockMetaStorage, BlockMetaStorageReader, BlockStorage,
    CycleErasStorage,
};
use tezos_api::ffi::{RpcRequest, RpcMethod, ApplyBlockRequest};
use tezos_messages::{protocol::{SupportedProtocol, UnsupportedProtocolError}, p2p::encoding::operations_for_blocks::OperationsForBlocksMessage};
use time::{Instant, Duration};

use super::{base_services::get_additional_data_or_fail, protocol};

/// A struct holding the reward values as mutez strings to be able to serialize BigInts
#[derive(Clone, Debug, Default, Serialize)]
pub struct CycleRewards {
    fees: String,
    baking_rewards: String,
    baking_bonuses: String,
    endorsement_rewards: String,
    sum: String,
}

#[derive(Clone, Debug, Default)]
pub struct CycleRewardsInt {
    fees: BigInt,
    baking_rewards: BigInt,
    baking_bonuses: BigInt,
    endorsement_rewards: BigInt,
    sum: BigInt,
}

impl From<CycleRewardsInt> for CycleRewards {
    fn from(num_ver: CycleRewardsInt) -> Self {
        Self {
            fees: num_ver.fees.to_string(),
            baking_rewards: num_ver.baking_rewards.to_string(),
            endorsement_rewards: num_ver.endorsement_rewards.to_string(),
            baking_bonuses: num_ver.baking_bonuses.to_string(),
            sum: num_ver.sum.to_string(),
        }
    }
}

// pub struct BalanceUpdate {

// }

// TODO: include legacy stuff?
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BalanceUpdateKind {
    Contract(ContractKind),
    Accumulator(AccumulatorKind),
    Freezer(FreezerKind),
    Minted(MintedKind),
    Burned,
    Commitment,
    Unknown,
}

impl Default for BalanceUpdateKind {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MintedKind {
    category: BalanceUpdateCategory,
    change: String,
    origin: BalanceUpdateOrigin,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AccumulatorKind {
    category: BalanceUpdateCategory,
    change: String,
    origin: BalanceUpdateOrigin,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ContractKind {
    contract: String,
    change: String,
    origin: BalanceUpdateOrigin,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FreezerKind {
    delegate: String,
    change: String,
    origin: BalanceUpdateOrigin,
    category: BalanceUpdateCategory,
}

// TODO: include legacy stuff?
// Note: We need to rename the variants because of "tezos case" variants.....
#[derive(Clone, Debug, Deserialize)]
pub enum BalanceUpdateCategory {
    #[serde(rename = "block fees")]
    BlockFees,
    #[serde(rename = "deposits")]
    Deposits,
    #[serde(rename = "nonce revelation rewards")]
    NonceRevelationRewards,
    #[serde(rename = "double signing evidence rewards")]
    DoubleSigningEvidenceRewards,
    #[serde(rename = "endorsing rewards")]
    EndorsingRewards,
    #[serde(rename = "baking rewards")]
    BakingRewards,
    #[serde(rename = "baking bonuses")]
    BakingBonuses,
    #[serde(rename = "storage fees")]
    StorageFees,
    #[serde(rename = "punishment")]
    Punishment,
    #[serde(rename = "lost endorsing rewards")]
    LostEndorsingRewards,
    #[serde(rename = "subsidy")]
    Subsidy,
    #[serde(rename = "burned")]
    Burned,
    #[serde(rename = "commitment")]
    Commitment,
    #[serde(rename = "bootstrap")]
    Bootstrap,
    #[serde(rename = "invoice")]
    Invoice,
    #[serde(rename = "minted")]
    Minted,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BalanceUpdateOrigin {
    Block,
    Migration,
    Subsidy,
    Simulation,
}

pub struct BlockFees {
    origin: BalanceUpdateOrigin,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DelegateInfo {
    full_balance: String,
    current_frozen_deposits: String,
    frozen_deposits: String,
    staking_balance: String,
    delegated_contracts: Vec<String>,
    delegated_balance: String,
    deactivated: bool,
    grace_period: i32,
    voting_power: i32,
}

#[derive(Clone, Debug, Default)]
pub struct DelegateInfoInt {
    full_balance: BigInt,
    current_frozen_deposits: BigInt,
    frozen_deposits: BigInt,
    staking_balance: BigInt,
    delegated_contracts: Vec<String>,
    delegated_balance: BigInt,
    deactivated: bool,
    grace_period: i32,
    voting_power: i32,

    // create delegated balances map
    delegator_balances: BTreeMap<Delegate, BigInt>, 
}

impl TryFrom<DelegateInfo> for DelegateInfoInt {
    type Error = anyhow::Error;

    fn try_from(value: DelegateInfo) -> Result<Self, Self::Error> {
        let delegate_info = DelegateInfoInt {
            full_balance: BigInt::from_str(&value.full_balance)?,
            current_frozen_deposits: BigInt::from_str(&value.current_frozen_deposits)?,
            frozen_deposits: BigInt::from_str(&value.frozen_deposits)?,
            staking_balance: BigInt::from_str(&value.staking_balance)?,
            delegated_balance: BigInt::from_str(&value.delegated_balance)?,
            delegated_contracts: value.delegated_contracts,
            deactivated: value.deactivated,
            grace_period: value.grace_period,
            voting_power: value.voting_power,
            delegator_balances: BTreeMap::new(),
        };
        Ok(delegate_info)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationKind {
    SeedNonceRevelation,
    DoubleBakingEvidence,
    DoublePreendorsementEvidence,
    DoubleEndorsementEvidence,
    ActivateAccount,
    Ballot,
}

#[derive(Clone, Debug, Deserialize)]
struct OperationMetadata {
    balance_updates: Vec<BalanceUpdateKind>,
}

#[derive(Clone, Debug, Deserialize)]
struct BareOperationKind {
    kind: OperationKind,
    metadata: OperationMetadata,
}

#[derive(Clone, Debug, Deserialize)]
struct OperationRepresentation {
    protocol: String,
    chain_id: String,
    hash: String,
    branch: String,
    contents: Vec<BareOperationKind>,
}

// TODO: should we use the concrete PublicKeyHash type?
pub type Delegate = String;
pub type Delegator = String;
pub type DelegateCycleRewards = BTreeMap<Delegate, CycleRewards>;

#[derive(Clone, Debug, Serialize)]
pub struct DelegateRewardDistribution {
    address: String,
    total_rewards: String,
    staking_balance: String,
    delegator_rewards: Vec<DelegatorInfo>,
}

impl DelegateRewardDistribution {
    fn new(address: String, total_rewards: String, staking_balance: String) -> Self {
        Self {
            address,
            total_rewards,
            staking_balance,
            delegator_rewards: Vec::new(),
        }
    }

    fn insert_delegator_reward(&mut self, delegator: &str, delegator_info: DelegatorInfo) {
        self.delegator_rewards.push(delegator_info);
    }
}

#[derive(Clone, Debug, Serialize)]
struct DelegatorInfo {
    address: String,
    balance: String,
    reward: String,
}

impl DelegatorInfo {
    fn new(address: String, balance: String, reward: String) -> Self {
        Self {
            address,
            balance,
            reward,
        }
    }
}

// To serialize bigint we use its string form
// pub type DelegateRewardDistribution = BTreeMap<Delegator, String>;
pub type CycleRewardDistribution = Vec<DelegateRewardDistribution>;

// TODO: create proper errors
pub(crate) async fn get_cycle_rewards(
    chain_id: &ChainId,
    env: &RpcServiceEnvironment,
    cycle_num: i32,
) -> Result<CycleRewardDistribution, anyhow::Error> {
    // TODO: get from constants
    const PRESERVED_CYCLES: i32 = 3;

    // TODO: get from query arg, default to 15?
    const COMISSION_PERCENTAGE: u16 = 15;

    let block_meta_storage = BlockMetaStorage::new(env.persistent_storage());
    let block_storage = BlockStorage::new(env.persistent_storage());
    let operations_storage = OperationsStorage::new(env.persistent_storage());

    let (current_head_level, current_head_hash) = if let Ok(shared_state) = env.state().read() {
        (
            shared_state.current_head().header.level(),
            shared_state.current_head().hash.clone(),
        )
    } else {
        anyhow::bail!("Cannot access current head")
    };

    let protocol_hash =
        &get_additional_data_or_fail(chain_id, &current_head_hash, env.persistent_storage())?
            .protocol_hash;
    let mut result: BTreeMap<Delegate, CycleRewardsInt> = BTreeMap::new();
    // Note: (Assumption, needs to be verifed) There is a bug in cycle era storage that won't save the era data on a new protocol change if there was no change
    match SupportedProtocol::try_from(protocol_hash)? {
        SupportedProtocol::Proto012 => {
            let saved_cycle_era_in_proto_hash =
                ProtocolHash::from_base58_check(&SupportedProtocol::Proto011.protocol_hash())?;
            let cycle_era = get_cycle_era(&saved_cycle_era_in_proto_hash, cycle_num, env)?;

            let (start, end) = cycle_range(&cycle_era, cycle_num);
            slog::crit!(env.log(), "CYCLE: {cycle_num} ({start} - {end})");

            // let start_hash = parse_block_hash_or_fail!(&chain_id, &start.to_string(), &env);
            let end_hash = parse_block_hash(chain_id, &end.to_string(), env)?;

            let mut blocks: Vec<(BlockHeaderWithHash, BlockJsonData, BlockAdditionalData, Vec<OperationsForBlocksMessage>)> = Vec::with_capacity(*cycle_era.blocks_per_cycle() as usize);

            // get all the data needed from storage
            for level in start..=end {
                let hash = parse_block_hash(chain_id, &level.to_string(), env)?;
                if let Some((block_header_with_hash, block_json_data)) = block_storage.get_with_json_data(&hash)? {
                    if let Some(block_additional_data) = block_meta_storage.get_additional_data(&block_header_with_hash.hash)? {
                        let operations_data = operations_storage.get_operations(&block_header_with_hash.hash)?;
                        blocks.push((block_header_with_hash, block_json_data, block_additional_data, operations_data));
                    }
                }
            }

            let mut connection = env.tezos_protocol_api().readable_connection().await?;
            for (block_header, block_json_data, block_additional_data, operations_data) in blocks {
                let response = connection
                    .apply_block_result_metadata(
                        block_header.header.context().clone(),
                        block_json_data.block_header_proto_metadata_bytes,
                        block_additional_data.max_operations_ttl().into(),
                        block_additional_data.protocol_hash.clone(),
                        block_additional_data.next_protocol_hash.clone(),
                    )
                    .await;

                let response = if let Ok(response) = response {
                    response
                } else {
                    continue;
                };

                let metadata: BlockMetadata =
                    serde_json::from_str(&response).unwrap_or_default();

                let converted_ops = ApplyBlockRequest::convert_operations(operations_data);

                // Optimalization: Deserialize the operations only when the anonymous validation pass is not empty
                // Further optimalization would be the ability to deserialize only one validation pass
                let block_operations: Option<BlockOperations> = if !converted_ops[2].is_empty() {
                    let response = connection
                        .apply_block_operations_metadata(
                            chain_id.clone(),
                            converted_ops,
                            block_json_data.operations_proto_metadata_bytes,
                            block_additional_data.protocol_hash.clone(),
                            block_additional_data.next_protocol_hash.clone(),
                        )
                        .await;

                    let response = if let Ok(response) = response {
                        response
                    } else {
                        continue;
                    };

                    Some(serde_json::from_str(&response).unwrap_or_default())
                } else {
                    None
                };

                if let Some(balance_updates) = metadata.get("balance_updates") {
                    if let Some(balance_updates_array) = balance_updates.as_array() {
                        // balance_updates_array.iter().map(|val| serde_json::from_value(val.clone()).unwrap_or_default()).collect()
                        for balance_update in balance_updates_array {
                            // deserialize
                            let balance_update: BalanceUpdateKind =
                                serde_json::from_value(balance_update.clone())
                                    .unwrap_or_default();
                            // if let BalanceUpdateKind::Contract(contract_updates) =
                            //     balance_update
                            // {
                                
                            // }
                            match balance_update {
                                BalanceUpdateKind::Contract(contract_updates) => {
                                    result
                                        .entry(contract_updates.contract.clone())
                                        .or_insert_with(CycleRewardsInt::default)
                                        .sum += BigInt::from_str(&contract_updates.change)?;
                                
                                    // DEBUG
                                    // if contract_updates.contract == "tz1ZNnVcwJk53UuBvMjAW7DF1UGXZqmShQSp" {
                                    //     slog::crit!(env.log(), "Level {} - Block hash {}: {:#?}", block_header.hash, block_header.header.level(), contract_updates)
                                    // }
                                }
                                // The contract balance_update subtracts the deposit, we just add back the deposited amount
                                BalanceUpdateKind::Freezer(freezer_update) => {
                                    if let BalanceUpdateCategory::Deposits = freezer_update.category {
                                        result
                                            .entry(freezer_update.delegate.clone())
                                            .or_insert_with(CycleRewardsInt::default)
                                            .sum += BigInt::from_str(&freezer_update.change)?;
                                        // DEBUG
                                        // if freezer_update.delegate == "tz1Qm727PrLHPme6gcz2Gg8YAXqUrq8oDhio" {
                                        //     slog::crit!(env.log(), "Deposit at level {}: {:#?}", block_header.header.level(), freezer_update)
                                        // }
                                    }
                                }
                                // DEBUG
                                // BalanceUpdateKind::Minted(minted_update) => {
                                //     if let BalanceUpdateCategory::NonceRevelationRewards = minted_update.category {
                                //         slog::crit!(env.log(), "Minted Nonce reward at level {}: {:#?}", block_header.header.level(), minted_update)
                                //     }
                                // }
                                _ => { /* Ignore other receipts */ }
                            }
                        }
                    } else {
                        anyhow::bail!("Balance updates not an array");
                    }
                } else {
                    anyhow::bail!("Balance updates not found");
                };

                // for val_pass in block_operations {
                //     for op in val_pass {
                //         slog::crit!(env.log(), "Ops: {:#?}", op.get());
                //     }
                // }

                if let Some(block_operations) = block_operations {
                    for operations in &block_operations[2] {
                        let operation: OperationRepresentation = serde_json::from_str(operations.get())?;
    
                        for content in operation.contents {
                            match content.kind {
                                OperationKind::DoubleBakingEvidence |
                                OperationKind::DoubleEndorsementEvidence |
                                OperationKind::DoublePreendorsementEvidence |
                                OperationKind::SeedNonceRevelation => {
                                    for balance_update in content.metadata.balance_updates {
                                        if let BalanceUpdateKind::Contract(contract_updates) = balance_update {
                                            result
                                                .entry(contract_updates.contract.clone())
                                                .or_insert_with(CycleRewardsInt::default)
                                                .sum += BigInt::from_str(&contract_updates.change)?;
                                        }
                                    }
                                }
                                _ => { /* Ignore */ }
                            }
                        }
                    }
                }
            }

            // trim all delegates that has 0 rewards (deposit changes could still occur after being inactive)
            // so only active delegates will be checked
            result.retain(|_, reward| {
                reward.sum != BigInt::from(0)
            });

            slog::crit!(env.log(), "REWARDS OK");

            // for the interogated cycle the delegate stuff was set at the end of current_cycle - PRESERVED_CYCLES - 1
            let frozen_cycle = cycle_num - PRESERVED_CYCLES - 1;
            let frozen_cycle_era = get_cycle_era(&saved_cycle_era_in_proto_hash, frozen_cycle, env)?;
            let (frozen_start, frozen_end) = cycle_range(&frozen_cycle_era, frozen_cycle);

            let frozen_end_hash = parse_block_hash(chain_id, &frozen_end.to_string(), env)?;

            let frozen_cycle_snapshot = get_routed_request(
                &format!("chains/main/blocks/{end}/context/selected_snapshot?cycle={cycle_num}"),
                // TODO: we need current head? more likely a specific block inside the cycle (snapshot)
                end_hash.clone(),
                env
            ).await?;

            slog::crit!(env.log(), "CYCLE SNAPSHOT RAW: {:#?}", frozen_cycle_snapshot);

            let frozen_cycle_snapshot = frozen_cycle_snapshot.trim_end_matches('\n').parse::<i32>()?;

            let frozen_snapshot_block = get_snapshot_block(frozen_start, frozen_cycle_snapshot);
            let frozen_snapshot_block_hash = parse_block_hash(chain_id, &frozen_snapshot_block.to_string(), env)?;
            slog::crit!(env.log(), "FROZEN BLOCK: {frozen_snapshot_block}");

            slog::crit!(env.log(), "FROZEN STUFF OK");

            // we need to get all the delegators for a specific delegate
            let mut delegate_info_map: BTreeMap<Delegate, DelegateInfoInt> = BTreeMap::new();
            let mut delegates_reward_distribution: CycleRewardDistribution = Vec::with_capacity(result.len());
            for (delegate, cycle_rewards) in result.iter() {
                let delegate_info_string = get_routed_request(
                    &format!("chains/main/blocks/{frozen_snapshot_block}/context/delegates/{delegate}"),
                    // TODO: we need current head? more likely a specific block inside the cycle (snapshot)
                    frozen_snapshot_block_hash.clone(),
                    env
                ).await?;
                let mut delegate_info: DelegateInfoInt = serde_json::from_str::<DelegateInfo>(&delegate_info_string)?.try_into()?;

                let mut reward_distributon = DelegateRewardDistribution::new(
                    delegate.clone(),
                    cycle_rewards.sum.to_string(),
                    delegate_info.staking_balance.to_string(),
                );

                slog::crit!(env.log(), "DELEGATE INFO OK");

                let delegators = delegate_info.delegated_contracts.clone();
                let mut delegator_balance_sum: BigInt = BigInt::new(Sign::Plus, vec![0]);
                for delegator in delegators {
                    // ignore the delegate itseld as it is part of the list
                    if &delegator == delegate {
                        continue;
                    }
                    let delegator_balance = get_routed_request(
                        &format!("chains/main/blocks/{frozen_snapshot_block}/context/raw/json/contracts/index/{delegator}/balance"),
                        // TODO: we need current head? more likely a specific block inside the cycle (snapshot)
                        frozen_snapshot_block_hash.clone(),
                        env
                    ).await?;
                    // slog::crit!(env.log(), "DELEGATOR BALANCE RAW: {:#?}", delegator_balance.trim_end_matches('\n').trim_matches('\"'));
                    let delegator_balance = delegator_balance.trim_end_matches('\n').trim_matches('\"').parse::<BigInt>().ok().unwrap_or_else(|| BigInt::new(Sign::Plus, vec![0]));
                    
                    let delegator_reward_share = get_delegator_reward_share(delegate_info.staking_balance.clone(), cycle_rewards.sum.clone(), delegator_balance.clone());
                    let delegator_info = DelegatorInfo::new(delegator.clone(), delegator_balance.to_string(), delegator_reward_share.to_string());
                    reward_distributon.insert_delegator_reward(&delegator, delegator_info);

                    delegator_balance_sum += delegator_balance.clone();
                    delegate_info.delegator_balances.insert(delegator, delegator_balance);
                }

                if delegate_info.delegated_balance == delegator_balance_sum {
                    slog::crit!(env.log(), "{}'s DELEGATOR BALANCES OK (total rewards {} - stake {})", delegate, cycle_rewards.sum, delegate_info.staking_balance.clone());
                } else {
                    slog::crit!(env.log(), "{}'s DELEGATOR BALANCES is off", delegate);
                    slog::crit!(env.log(), "Actual: {} - Summed: {} - Diff: {}", delegate_info.delegated_balance.clone(), delegator_balance_sum.clone(), (delegator_balance_sum - delegate_info.delegated_balance.clone()).abs());
                    slog::crit!(env.log(), "{}'s delegators: {:#?}", delegate, delegate_info.delegator_balances);
                }

                // DEBUG
                // if delegate == "tz1Qm727PrLHPme6gcz2Gg8YAXqUrq8oDhio" {
                //     slog::crit!(env.log(), "{}'s delegators: {:#?}", delegate, delegate_info.delegator_balances);
                // }
                delegate_info_map.insert(delegate.to_string(), delegate_info);
                delegates_reward_distribution.push(reward_distributon);
                // slog::crit!(env.log(), "{}'s delegators: {:#?}", delegate, delegate_info.delegated_contracts);
            }
            Ok(delegates_reward_distribution)
        }
        _ => {
            Err(UnsupportedProtocolError {
                protocol: protocol_hash.to_string(),
            }
            .into())
        }
    }

    // Ok(result
    //     .into_iter()
    //     .map(|(delegate, rewards)| (delegate, rewards.into()))
    //     .collect())
}

fn get_cycle_era(protocol_hash: &ProtocolHash, cycle_num: i32, env: &RpcServiceEnvironment) -> Result<CycleEra, anyhow::Error> {
    if let Some(eras) = CycleErasStorage::new(env.persistent_storage())
        .get(&protocol_hash)?
    {
        if let Some(era) = eras.into_iter().find(|era| era.first_cycle() < &cycle_num) {
            Ok(era)
        } else {
            anyhow::bail!("No matching cycle era found")
        }
    } else {
        anyhow::bail!("No saved cycle eras found for protocol")
    }
}

fn get_snapshot_block(start_block_level: i32, snapshot_index: i32) -> i32 {
    // TODO: get from constants
    const BLOCKS_PER_STAKE_SNAPSHOT: i32 = 256;

    start_block_level + (snapshot_index + 1) * BLOCKS_PER_STAKE_SNAPSHOT - 1
}

fn cycle_range(era: &CycleEra, cycle_num: i32) -> (i32, i32) {
    let cycle_offset = cycle_num - *era.first_cycle();

    let start = *era.first_level() + cycle_offset * *era.blocks_per_cycle();
    let end = start + *era.blocks_per_cycle() - 1;

    (start, end)
}

async fn get_routed_request(path: &str, block_hash: BlockHash, env: &RpcServiceEnvironment) -> Result<String, anyhow::Error> {
    let meth = RpcMethod::GET;

    let body = String::from("");

    let req = RpcRequest {
        body,
        context_path: String::from(path.trim_end_matches('/')),
        meth,
        content_type: None,
        accept: None
    };
    let chain_id = parse_chain_id(MAIN_CHAIN_ID, &env)?;

    let res = protocol::call_protocol_rpc(MAIN_CHAIN_ID, chain_id, block_hash, req, &env).await?;

    Ok(res.1.clone())
}

// TODO: comisson
fn get_delegator_reward_share(staking_balance: BigInt, delegate_total_reward: BigInt, delegator_balance: BigInt) -> BigInt {
    let share = BigRational::new(delegator_balance, staking_balance);

    let reward = BigRational::from(delegate_total_reward) * share;

    reward.round().to_integer()
}