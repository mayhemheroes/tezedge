// Copyright (c) SimpleStaking, Viable Systems and Tezedge Contributors
// SPDX-License-Identifier: MIT

use std::{fmt, str};

use serde::{Deserialize, Serialize};

use crypto::hash::{
    BlockHash, BlockPayloadHash, ContextHash, NonceHash, OperationHash,
    OperationListListHash, ProtocolHash, Signature,
};
use tezos_encoding::{enc::BinWriter, encoding::HasEncoding, nom::NomReader, types::SizedBytes};
use tezos_messages::protocol::proto_012::operation::EndorsementOperation;

// signature watermark: 0x11 | chain_id
#[derive(BinWriter, HasEncoding, NomReader, Serialize, Clone, Debug)]
pub struct ProtocolBlockHeader {
    pub payload_hash: BlockPayloadHash,
    pub payload_round: i32,
    pub proof_of_work_nonce: SizedBytes<8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_nonce_hash: Option<NonceHash>,
    pub liquidity_baking_escape_vote: bool,
    pub signature: Signature,
}

#[derive(Deserialize, Debug)]
pub struct Constants {
    pub nonce_length: usize,
    pub blocks_per_cycle: u32,
    pub consensus_committee_size: u32,
    pub minimal_block_delay: String,
    pub delay_increment_per_round: String,
    pub blocks_per_commitment: u32,
    pub proof_of_work_threshold: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OperationSimple {
    pub branch: BlockHash,
    pub contents: Vec<serde_json::Value>,
    #[serde(default)]
    pub signature: Option<Signature>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<OperationHash>,
    #[serde(default)]
    pub protocol: Option<ProtocolHash>,
}

pub enum OperationKind {
    Preendorsement(EndorsementOperation),
    Endorsement(EndorsementOperation),
    Votes,
    Anonymous,
    Managers,
}

impl fmt::Debug for OperationSimple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Operation")
            .field("branch", &self.branch)
            .field("hash", &self.hash)
            .field("protocol", &self.protocol)
            .field("contents", &serde_json::to_string(&self.contents))
            .field("signature", &self.signature)
            .finish()
    }
}

impl OperationSimple {
    pub fn kind(&self) -> Option<OperationKind> {
        fn op_kind(c: &serde_json::Value) -> Option<&str> {
            c.as_object()?.get("kind")?.as_str()
        }

        let c = self.contents.first()?.clone();

        match op_kind(&c)? {
            "preendorsement" => {
                let mut c = c;
                let c_obj = c.as_object_mut()?;
                c_obj.remove("kind");
                c_obj.remove("metadata");
                serde_json::from_value(c)
                    .ok()
                    .map(OperationKind::Preendorsement)
            }
            "endorsement" => {
                let mut c = c;
                let c_obj = c.as_object_mut()?;
                c_obj.remove("kind");
                c_obj.remove("metadata");
                serde_json::from_value(c)
                    .ok()
                    .map(OperationKind::Endorsement)
            }
            "failing_noop" => None,
            "proposals" | "ballot" => Some(OperationKind::Votes),
            "seed_nonce_revelation"
            | "double_preendorsement_evidence"
            | "double_endorsement_evidence"
            | "double_baking_evidence"
            | "activate_account" => Some(OperationKind::Anonymous),
            _ => Some(OperationKind::Managers),
        }
    }
}

pub struct Block {
    pub hash: BlockHash,
    pub level: i32,
    pub proto: u8,
    pub predecessor: BlockHash,
    pub timestamp: u64,
    pub validation_pass: u8,
    pub operations_hash: OperationListListHash,
    pub fitness: Vec<String>,
    pub context: ContextHash,
    pub payload_hash: BlockPayloadHash,
    pub payload_round: i32,
    pub round: i32,

    pub transition: bool,
    pub operations: Vec<Vec<OperationSimple>>,
}

pub enum Event {
    Block(Block),
    Operations(Vec<OperationSimple>),
    Tick,
}
