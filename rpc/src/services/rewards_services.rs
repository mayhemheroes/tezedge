use std::{collections::BTreeMap, str::FromStr};

use crate::{
    helpers::{parse_block_hash, BlockMetadata, RpcServiceError},
    parse_block_hash_or_fail, RpcServiceEnvironment,
};
use crypto::hash::{ChainId, ProtocolHash};
use num::BigInt;
use serde::{Deserialize, Serialize};
use storage::BlockStorageReader;
use storage::{
    cycle_eras_storage::CycleEra, BlockMetaStorage, BlockMetaStorageReader, BlockStorage,
    CycleErasStorage,
};
use tezos_messages::protocol::{SupportedProtocol, UnsupportedProtocolError};

use super::base_services::get_additional_data_or_fail;

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
    Freezer,
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

// TODO: should we use the concrete PublicKeyHash type?
pub type Delegate = String;
pub type DelegateCycleRewards = BTreeMap<Delegate, CycleRewards>;

// TODO: create proper errors
pub(crate) async fn get_cycle_rewards(
    chain_id: &ChainId,
    env: &RpcServiceEnvironment,
    cycle_num: i32,
) -> Result<DelegateCycleRewards, anyhow::Error> {
    let block_meta_storage = BlockMetaStorage::new(env.persistent_storage());
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
            let cycle_era = if let Some(eras) = CycleErasStorage::new(env.persistent_storage())
                .get(&saved_cycle_era_in_proto_hash)?
            {
                if let Some(era) = eras.into_iter().find(|era| era.first_cycle() < &cycle_num) {
                    era
                } else {
                    anyhow::bail!("No matching cycle era found")
                }
            } else {
                anyhow::bail!("No saved cycle eras found for protocol")
            };

            let (start, end) = cycle_range(&cycle_era, cycle_num);

            // let start_hash = parse_block_hash_or_fail!(&chain_id, &start.to_string(), &env);
            let start_hash = parse_block_hash(chain_id, &start.to_string(), env)?;

            let blocks = BlockStorage::new(env.persistent_storage())
                .get_multiple_with_json_data(&start_hash, *cycle_era.blocks_per_cycle() as usize)?;

            let mut connection = env.tezos_protocol_api().readable_connection().await?;
            for (block_header, block_json_data) in blocks {
                if let Some(block_additional_data) =
                    block_meta_storage.get_additional_data(&block_header.hash)?
                {
                    let response = connection
                        .apply_block_result_metadata(
                            block_header.header.context().clone(),
                            block_json_data.block_header_proto_metadata_bytes,
                            block_additional_data.max_operations_ttl().into(),
                            block_additional_data.protocol_hash,
                            block_additional_data.next_protocol_hash,
                        )
                        .await;

                    let response = if let Ok(response) = response {
                        response
                    } else {
                        continue;
                    };

                    let metadata: BlockMetadata =
                        serde_json::from_str(&response).unwrap_or_default();
                    // let cycle_position = if let Some(level) = metadata.get("level") {
                    //     level["cycle_position"].as_i64()
                    // } else if let Some(level) = metadata.get("level_info") {
                    //     level["cycle_position"].as_i64()
                    // } else {
                    //     None
                    // };
                    if let Some(balance_updates) = metadata.get("balance_updates") {
                        if let Some(balance_updates_array) = balance_updates.as_array() {
                            // balance_updates_array.iter().map(|val| serde_json::from_value(val.clone()).unwrap_or_default()).collect()
                            for balance_update in balance_updates_array {
                                // deserialize
                                let balance_update: BalanceUpdateKind =
                                    serde_json::from_value(balance_update.clone())
                                        .unwrap_or_default();
                                if let BalanceUpdateKind::Contract(contract_updates) =
                                    balance_update
                                {
                                    result
                                        .entry(contract_updates.contract)
                                        .or_insert_with(CycleRewardsInt::default)
                                        .sum += BigInt::from_str(&contract_updates.change)?;
                                }
                            }
                        } else {
                            anyhow::bail!("Balance updates not an array");
                        }
                    } else {
                        anyhow::bail!("Balance updates not found");
                    };

                    // result.push(SlimBlockData {
                    //     level: block_header.header.level(),
                    //     block_hash: block_header.hash.to_base58_check(),
                    //     timestamp: block_header.header.timestamp().to_string(),
                    //     cycle_position,
                    // });
                }
            }
        }
        _ => {
            return Err(UnsupportedProtocolError {
                protocol: protocol_hash.to_string(),
            }
            .into())
        }
    }

    Ok(result
        .into_iter()
        .map(|(delegate, rewards)| (delegate, rewards.into()))
        .collect())
}

fn cycle_range(era: &CycleEra, cycle_num: i32) -> (i32, i32) {
    let cycle_offset = cycle_num - *era.first_cycle();

    let start = *era.first_level() + cycle_offset * *era.blocks_per_cycle();
    let end = start + *era.blocks_per_cycle() - 1;

    (start, end)
}
