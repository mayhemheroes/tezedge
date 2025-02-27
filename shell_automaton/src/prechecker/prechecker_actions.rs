// Copyright (c) SimpleStaking, Viable Systems and Tezedge Contributors
// SPDX-License-Identifier: MIT

use std::sync::Arc;

use crypto::hash::{BlockPayloadHash, OperationHash, ProtocolHash};
use storage::BlockHeaderWithHash;
use tezos_messages::p2p::encoding::operation::Operation;

use crate::{EnablingCondition, State};

use super::{EndorsementBranch, PrecheckerOperationState};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerCurrentHeadUpdateAction {
    pub head: Arc<BlockHeaderWithHash>,
    pub protocol: ProtocolHash,
    pub payload_hash: Option<BlockPayloadHash>,
}

impl EnablingCondition<State> for PrecheckerCurrentHeadUpdateAction {
    fn is_enabled(&self, _state: &State) -> bool {
        true
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerStoreEndorsementBranchAction {
    pub endorsement_branch: Option<EndorsementBranch>,
}

impl EnablingCondition<State> for PrecheckerStoreEndorsementBranchAction {
    fn is_enabled(&self, _state: &State) -> bool {
        true
    }
}

macro_rules! from_hash_ref {
    ($action:ident) => {
        impl From<&OperationHash> for $action {
            fn from(source: &OperationHash) -> Self {
                Self {
                    hash: source.clone(),
                }
            }
        }
    };
}

macro_rules! from_hash {
    ($action:ident) => {
        impl From<OperationHash> for $action {
            fn from(source: OperationHash) -> Self {
                Self { hash: source }
            }
        }
    };
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerPrecheckOperationAction {
    pub hash: OperationHash,
    pub operation: Operation,
    pub proto: u8,
}

impl EnablingCondition<State> for PrecheckerPrecheckOperationAction {
    fn is_enabled(&self, state: &State) -> bool {
        state.current_head.is_precheckable()
            && (self.proto == state.prechecker.proto || self.proto == state.prechecker.proto + 1)
            && !state.prechecker.operations.contains_key(&self.hash)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerPrecheckDelayedOperationAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerPrecheckDelayedOperationAction {
    fn is_enabled(&self, _state: &State) -> bool {
        true
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerProtocolSupportedAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerProtocolSupportedAction {
    fn is_enabled(&self, state: &State) -> bool {
        matches!(
            state.prechecker.state(&self.hash),
            Some(PrecheckerOperationState::Init { proto }) if proto == &state.prechecker.proto
        )
    }
}

from_hash!(PrecheckerProtocolSupportedAction);
from_hash_ref!(PrecheckerProtocolSupportedAction);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerDecodeOperationAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerDecodeOperationAction {
    fn is_enabled(&self, state: &State) -> bool {
        let prechecker_state = &state.prechecker;
        matches!(
            prechecker_state.state(&self.hash),
            Some(PrecheckerOperationState::Supported { .. })
        )
    }
}

from_hash_ref!(PrecheckerDecodeOperationAction);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerCategorizeOperationAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerCategorizeOperationAction {
    fn is_enabled(&self, state: &State) -> bool {
        matches!(
            state.prechecker.state(&self.hash),
            Some(PrecheckerOperationState::Decoded { .. })
        )
    }
}

from_hash_ref!(PrecheckerCategorizeOperationAction);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerProtocolNeededAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerProtocolNeededAction {
    fn is_enabled(&self, state: &State) -> bool {
        matches!(
            state.prechecker.state(&self.hash),
            Some(PrecheckerOperationState::ProtocolNeeded { .. })
        )
    }
}

from_hash_ref!(PrecheckerProtocolNeededAction);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerValidateOperationAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerValidateOperationAction {
    fn is_enabled(&self, state: &State) -> bool {
        matches!(
            state.prechecker.state(&self.hash),
            Some(PrecheckerOperationState::TenderbakeConsensus { .. })
                | Some(PrecheckerOperationState::TenderbakePendingRights { .. })
        )
    }
}

from_hash!(PrecheckerValidateOperationAction);
from_hash_ref!(PrecheckerValidateOperationAction);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerOperationValidatedAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerOperationValidatedAction {
    fn is_enabled(&self, state: &State) -> bool {
        state
            .prechecker
            .state(&self.hash)
            .map_or(false, PrecheckerOperationState::is_result)
    }
}

from_hash_ref!(PrecheckerOperationValidatedAction);

#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrecheckerErrorAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerErrorAction {
    fn is_enabled(&self, state: &State) -> bool {
        state
            .prechecker
            .operations
            .get(&self.hash)
            .map_or(false, Result::is_err)
    }
}

from_hash_ref!(PrecheckerErrorAction);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerProtocolActivationAction {}

impl EnablingCondition<State> for PrecheckerProtocolActivationAction {
    fn is_enabled(&self, state: &State) -> bool {
        state
            .current_head
            .get()
            .map_or(false, |head| head.header.proto() != state.prechecker.proto)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerCacheDelayedOperationAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerCacheDelayedOperationAction {
    fn is_enabled(&self, state: &State) -> bool {
        state
            .prechecker
            .state(&self.hash)
            .map_or(false, |op_state| op_state.caching_level().is_some())
    }
}

from_hash_ref!(PrecheckerCacheDelayedOperationAction);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(fuzzcheck::DefaultMutator))]
pub struct PrecheckerPruneOperationAction {
    pub hash: OperationHash,
}

impl EnablingCondition<State> for PrecheckerPruneOperationAction {
    fn is_enabled(&self, state: &State) -> bool {
        state
            .prechecker
            .state(&self.hash)
            .map_or(false, PrecheckerOperationState::is_result)
    }
}

from_hash_ref!(PrecheckerPruneOperationAction);
from_hash!(PrecheckerPruneOperationAction);
