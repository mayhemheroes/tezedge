// Copyright (c) SimpleStaking, Viable Systems and Tezedge Contributors
// SPDX-License-Identifier: MIT

use serial_test::serial;

use crypto::hash::{ChainId, OperationHash};
use tezos_api::ffi::{
    ApplyBlockRequest, ApplyBlockResponse, BeginConstructionRequest, ClassifiedOperation,
    OperationClassification, ValidateOperationRequest, ValidateOperationResult,
};

use tezos_interop::apply_encoded_message;
use tezos_messages::p2p::binary_message::{BinaryRead, MessageHash};
use tezos_messages::p2p::encoding::prelude::*;
use tezos_protocol_ipc_messages::{NodeMessage, ProtocolMessage};

mod common;

#[test]
#[serial]
fn test_begin_construction() -> Result<(), anyhow::Error> {
    common::init_test_runtime();

    // init empty context for test
    let (chain_id, genesis_block_header, ..) = common::init_test_protocol_context(
        "mempool_test_storage_01",
        test_data_protocol_v1::tezos_network(),
    );

    // apply block 1 and block 2
    let (last_block, _apply_block_result) = apply_blocks_1_2(&chain_id, genesis_block_header);
    let predecessor_hash = last_block
        .hash()
        .as_ref()
        .unwrap()
        .as_slice()
        .try_into()
        .unwrap();

    // let's initialize prevalidator for current head
    let prevalidator = apply_encoded_message(ProtocolMessage::BeginConstruction(
        BeginConstructionRequest {
            chain_id: chain_id.clone(),
            predecessor: last_block,
            predecessor_hash,
            protocol_data: None,
        },
    ))
    .unwrap();
    let prevalidator = expect_response!(BeginConstructionResult, prevalidator)?;
    assert_eq!(prevalidator.chain_id, chain_id);

    let operation =
        test_data_protocol_v1::operation_from_hex(test_data_protocol_v1::OPERATION_LEVEL_3);
    let operation_hash = operation.message_typed_hash::<OperationHash>().unwrap();

    let result = apply_encoded_message(ProtocolMessage::ValidateOperation(
        ValidateOperationRequest {
            prevalidator,
            operation_hash,
            operation,
            include_operation_data_json: true,
        },
    ))
    .unwrap();
    let result = expect_response!(ValidateOperationResponse, result)?;
    assert_eq!(result.prevalidator.chain_id, chain_id);
    assert!(matches!(
        result.result,
        ValidateOperationResult::Classified(ClassifiedOperation {
            classification: OperationClassification::Applied,
            ..
        })
    ));

    Ok(())
}

fn apply_blocks_1_2(
    chain_id: &ChainId,
    genesis_block_header: BlockHeader,
) -> (BlockHeader, ApplyBlockResponse) {
    // apply first block - level 1
    let apply_block_result =
        apply_encoded_message(ProtocolMessage::ApplyBlockCall(ApplyBlockRequest {
            chain_id: chain_id.clone(),
            block_header: BlockHeader::from_bytes(
                hex::decode(test_data_protocol_v1::BLOCK_HEADER_LEVEL_1).unwrap(),
            )
            .unwrap(),
            pred_header: genesis_block_header,
            operations: ApplyBlockRequest::convert_operations(
                test_data_protocol_v1::block_operations_from_hex(
                    test_data_protocol_v1::BLOCK_HEADER_HASH_LEVEL_1,
                    test_data_protocol_v1::block_header_level1_operations(),
                ),
            ),
            max_operations_ttl: 0,
            predecessor_block_metadata_hash: None,
            predecessor_ops_metadata_hash: None,
        }))
        .unwrap();
    let apply_block_result = expect_response!(ApplyBlockResult, apply_block_result).unwrap();
    assert_eq!(
        test_data_protocol_v1::context_hash(
            test_data_protocol_v1::BLOCK_HEADER_LEVEL_1_CONTEXT_HASH
        ),
        apply_block_result.context_hash
    );
    assert_eq!(1, apply_block_result.max_operations_ttl);

    // apply second block - level 2
    let apply_block_result =
        apply_encoded_message(ProtocolMessage::ApplyBlockCall(ApplyBlockRequest {
            chain_id: chain_id.clone(),
            block_header: BlockHeader::from_bytes(
                hex::decode(test_data_protocol_v1::BLOCK_HEADER_LEVEL_2).unwrap(),
            )
            .unwrap(),
            pred_header: BlockHeader::from_bytes(
                hex::decode(test_data_protocol_v1::BLOCK_HEADER_LEVEL_1).unwrap(),
            )
            .unwrap(),
            operations: ApplyBlockRequest::convert_operations(
                test_data_protocol_v1::block_operations_from_hex(
                    test_data_protocol_v1::BLOCK_HEADER_HASH_LEVEL_2,
                    test_data_protocol_v1::block_header_level2_operations(),
                ),
            ),
            max_operations_ttl: apply_block_result.max_operations_ttl,
            predecessor_block_metadata_hash: apply_block_result.block_metadata_hash,
            predecessor_ops_metadata_hash: None,
        }))
        .unwrap();
    let apply_block_result = expect_response!(ApplyBlockResult, apply_block_result).unwrap();
    assert_eq!(
        test_data_protocol_v1::context_hash(
            test_data_protocol_v1::BLOCK_HEADER_LEVEL_2_CONTEXT_HASH
        ),
        apply_block_result.context_hash
    );
    assert_eq!(
        "lvl 2, fit 1:1, prio 8, 0 ops",
        &apply_block_result.validation_result_message
    );
    assert_eq!(2, apply_block_result.max_operations_ttl);

    (
        BlockHeader::from_bytes(hex::decode(test_data_protocol_v1::BLOCK_HEADER_LEVEL_2).unwrap())
            .unwrap(),
        apply_block_result,
    )
}

/// Test data for protocol_v1 like 008 edo
mod test_data_protocol_v1 {
    use std::convert::TryFrom;

    use crypto::hash::{BlockHash, ContextHash};
    use tezos_api::environment::TezosEnvironmentConfiguration;
    use tezos_context_api::{GenesisChain, PatchContext, ProtocolOverrides};
    use tezos_messages::p2p::binary_message::BinaryRead;
    use tezos_messages::p2p::encoding::prelude::*;

    pub fn tezos_network() -> TezosEnvironmentConfiguration {
        TezosEnvironmentConfiguration {
            genesis: GenesisChain {
                time: "2020-11-30T12:00:00Z".to_string(),
                block: "BLockGenesisGenesisGenesisGenesisGenesis2431bbUwV2a".to_string(),
                protocol: "PtYuensgYBb3G3x1hLLbCmcav8ue8Kyd2khADcL5LsT5R1hcXex".to_string(),
            },
            bootstrap_lookup_addresses: vec![
                "51.75.246.56:9733".to_string(),
                "edonet.tezos.co.il".to_string(),
                "46.245.179.161:9733".to_string(),
                "edonet.smartpy.io".to_string(),
                "188.40.128.216:29732".to_string(),
                "51.79.165.131".to_string(),
                "edonet.boot.tezostaquito.io".to_string(),
                "95.216.228.228:9733".to_string(),
            ],
            version: "TEZOS_EDONET_2020-11-30T12:00:00Z".to_string(),
            protocol_overrides: ProtocolOverrides {
                user_activated_upgrades: vec![],
                user_activated_protocol_overrides: vec![],
            },
            enable_testchain: true,
            patch_context_genesis_parameters: Some(PatchContext {
                key: "sandbox_parameter".to_string(),
                json: r#"{ "genesis_pubkey": "edpkugeDwmwuwyyD3Q5enapgEYDxZLtEUFFSrvVwXASQMVEqsvTqWu" }"#.to_string(),
            }),
        }
    }

    pub fn context_hash(hash: &str) -> ContextHash {
        ContextHash::from_base58_check(hash).unwrap()
    }

    // BLUzCt33hGwAsT4UdPXgqH2MjEZErpPfo5nL4rtQR5dStpixNrA
    pub const BLOCK_HEADER_HASH_LEVEL_1: &str =
        "6581e44bfff3b54e53e86b3587f7af579c12210a17f066f768aaf832c02c01c2";
    pub const BLOCK_HEADER_LEVEL_1: &str = include_str!("resources/edo_block_header_level1.bytes");
    pub const BLOCK_HEADER_LEVEL_1_CONTEXT_HASH: &str =
        "CoWT8jk3H1pHXz96JF7ke4SX3PVB9GZJCnDVf1Y5TSHvtNBsPaPT";

    pub fn block_header_level1_operations() -> Vec<Vec<String>> {
        vec![]
    }

    // BM5yiDGdmoubVFvnnU8DUtLPi48gsoqrwrJwt8yUkxa1NqZX5vt
    pub const BLOCK_HEADER_HASH_LEVEL_2: &str =
        "b4f5b6d362cdc21bebd5ec7fc829c5434ab316094a78ad05df93636ea7b1d3ee";
    pub const BLOCK_HEADER_LEVEL_2: &str = "00000002016581e44bfff3b54e53e86b3587f7af579c12210a17f066f768aaf832c02c01c2000000005fc4eaa404683625c2445a4e9564bf710c5528fd99a7d150d2a2a323bc22ff9e2710da4f6d000000110000000101000000080000000000000001fdb0f38fd96590f60cd5bad0b124c5db4c0c38ebad383eac852ca0eac27586120008c4907703fa490000009a05ac1d212b2b70b2d316e36cae41f63b0613483ae7e98cc12e7c8b45c6aaa52c68620caac9748d5116097756f7ae5bf217f095fb9750510d3b3045dbacd95d";
    pub const BLOCK_HEADER_LEVEL_2_CONTEXT_HASH: &str =
        "CoWa34JgZcJhQfSE9wugHiedhgW9vSaAscMTnfsF666oD26iGZKm";

    pub fn block_header_level2_operations() -> Vec<Vec<String>> {
        vec![vec![], vec![], vec![], vec![]]
    }

    pub const OPERATION_LEVEL_3: &str = "b4f5b6d362cdc21bebd5ec7fc829c5434ab316094a78ad05df93636ea7b1d3ee0000000002a521edcd56c091ebdeef3fde38c4d44e5cb100f0da703e567cdd07a667fedd312fb9a312261eb161ba8455e81f1ce70da1232a7ed4c2dc6583fb092aa0dd93c3";

    pub fn operation_from_hex(bytes: &str) -> Operation {
        Operation::from_bytes(hex::decode(bytes).unwrap()).unwrap()
    }

    pub fn block_operations_from_hex(
        block_hash: &str,
        hex_operations: Vec<Vec<String>>,
    ) -> Vec<OperationsForBlocksMessage> {
        hex_operations
            .into_iter()
            .map(|bo| {
                let ops = bo
                    .into_iter()
                    .map(|op| Operation::from_bytes(hex::decode(op).unwrap()).unwrap())
                    .collect();
                OperationsForBlocksMessage::new(
                    OperationsForBlock::new(
                        BlockHash::try_from(hex::decode(block_hash).unwrap()).unwrap(),
                        4,
                    ),
                    Path::op(),
                    ops,
                )
            })
            .collect()
    }
}
