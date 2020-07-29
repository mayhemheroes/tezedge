// Copyright (c) SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

use std::{io, thread};
use std::cell::RefCell;
use std::convert::AsRef;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use failure::Fail;
use getset::{CopyGetters, Getters};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use slog::{crit, debug, info, Level, Logger};
use strum_macros::IntoStaticStr;
use wait_timeout::ChildExt;

use crypto::hash::{ChainId, ContextHash, ProtocolHash};
use ipc::*;
use tezos_api::environment::TezosEnvironmentConfiguration;
use tezos_api::ffi::*;
use tezos_api::identity::Identity;
use tezos_context::channel::{context_receive, context_send, ContextAction};

use crate::protocol::*;

lazy_static! {
    /// Ww need to have multiple multiple FFI runtimes, and runtime needs to have initialized protocol context,
    /// but there are some limitations,
    /// e.g.: if we start readonly context with empty context directory, it fails in FFI runtime in irmin initialization,
    /// so we need to control this initialization on application level,
    ///
    /// in application we can have multiple threads, which tries to call init_protocol (read or write),
    /// so this lock ensures, that in the whole application at least one 'write init_protocol_context' was successfull, which means,
    /// that FFI context has created required files, so after that we can let continue other threads to initialize readonly context
    ///
    /// see also: test_mutliple_protocol_runners_with_one_write_multiple_read_init_context
    static ref AT_LEAST_ONE_WRITE_PROTOCOL_CONTEXT_WAS_SUCCESS_AT_FIRST_LOCK: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
}

/// This command message is generated by tezedge node and is received by the protocol runner.
#[derive(Serialize, Deserialize, Debug, IntoStaticStr)]
enum ProtocolMessage {
    ApplyBlockCall(ApplyBlockRequest),
    BeginConstructionCall(BeginConstructionRequest),
    ValidateOperationCall(ValidateOperationRequest),
    ProtocolJsonRpcCall(ProtocolJsonRpcRequest),
    HelpersPreapplyOperationsCall(ProtocolJsonRpcRequest),
    HelpersPreapplyBlockCall(ProtocolJsonRpcRequest),
    ChangeRuntimeConfigurationCall(TezosRuntimeConfiguration),
    InitProtocolContextCall(InitProtocolContextParams),
    GenesisResultDataCall(GenesisResultDataParams),
    GenerateIdentity(GenerateIdentityParams),
    ShutdownCall,
}

#[derive(Serialize, Deserialize, Debug)]
struct InitProtocolContextParams {
    storage_data_dir: String,
    genesis: GenesisChain,
    genesis_max_operations_ttl: u16,
    protocol_overrides: ProtocolOverrides,
    commit_genesis: bool,
    enable_testchain: bool,
    readonly: bool,
    patch_context: Option<PatchContext>,
}

#[derive(Serialize, Deserialize, Debug)]
struct GenesisResultDataParams {
    genesis_context_hash: ContextHash,
    chain_id: ChainId,
    genesis_protocol_hash: ProtocolHash,
    genesis_max_operations_ttl: u16,
}

#[derive(Serialize, Deserialize, Debug)]
struct GenerateIdentityParams {
    expected_pow: f64,
}

/// This event message is generated as a response to the `ProtocolMessage` command.
#[derive(Serialize, Deserialize, Debug, IntoStaticStr)]
enum NodeMessage {
    ApplyBlockResult(Result<ApplyBlockResponse, ApplyBlockError>),
    BeginConstructionResult(Result<PrevalidatorWrapper, BeginConstructionError>),
    ValidateOperationResponse(Result<ValidateOperationResponse, ValidateOperationError>),
    JsonRpcResponse(Result<JsonRpcResponse, ProtocolRpcError>),
    ChangeRuntimeConfigurationResult(Result<(), TezosRuntimeConfigurationError>),
    InitProtocolContextResult(Result<InitProtocolContextResult, TezosStorageInitError>),
    CommitGenesisResultData(Result<CommitGenesisResult, GetDataError>),
    GenerateIdentityResult(Result<Identity, TezosGenerateIdentityError>),
    ShutdownResult,
}

/// Empty message
#[derive(Serialize, Deserialize, Debug)]
struct NoopMessage;

pub fn process_protocol_events<P: AsRef<Path>>(socket_path: P) -> Result<(), IpcError> {
    let ipc_client: IpcClient<NoopMessage, ContextAction> = IpcClient::new(socket_path);
    let (_, mut tx) = ipc_client.connect()?;
    while let Ok(action) = context_receive() {
        tx.send(&action)?;
        if let ContextAction::Shutdown = action {
            break;
        }
    }

    Ok(())
}

/// Establish connection to existing IPC endpoint (which was created by tezedge node).
/// Begin receiving commands from the tezedge node until `ShutdownCall` command is received.
pub fn process_protocol_commands<Proto: ProtocolApi, P: AsRef<Path>>(socket_path: P) -> Result<(), IpcError> {
    let ipc_client: IpcClient<ProtocolMessage, NodeMessage> = IpcClient::new(socket_path);
    let (mut rx, mut tx) = ipc_client.connect()?;
    while let Ok(cmd) = rx.receive() {
        match cmd {
            ProtocolMessage::ApplyBlockCall(request) => {
                let res = Proto::apply_block(request);
                tx.send(&NodeMessage::ApplyBlockResult(res))?;
            }
            ProtocolMessage::BeginConstructionCall(request) => {
                let res = Proto::begin_construction(request);
                tx.send(&NodeMessage::BeginConstructionResult(res))?;
            }
            ProtocolMessage::ValidateOperationCall(request) => {
                let res = Proto::validate_operation(request);
                tx.send(&NodeMessage::ValidateOperationResponse(res))?;
            }
            ProtocolMessage::ProtocolJsonRpcCall(request) => {
                let res = Proto::call_protocol_json_rpc(request);
                tx.send(&NodeMessage::JsonRpcResponse(res))?;
            }
            ProtocolMessage::HelpersPreapplyOperationsCall(request) => {
                let res = Proto::helpers_preapply_operations(request);
                tx.send(&NodeMessage::JsonRpcResponse(res))?;
            }
            ProtocolMessage::HelpersPreapplyBlockCall(request) => {
                let res = Proto::helpers_preapply_block(request);
                tx.send(&NodeMessage::JsonRpcResponse(res))?;
            }
            ProtocolMessage::ChangeRuntimeConfigurationCall(params) => {
                let res = Proto::change_runtime_configuration(params);
                tx.send(&NodeMessage::ChangeRuntimeConfigurationResult(res))?;
            }
            ProtocolMessage::InitProtocolContextCall(params) => {
                let res = Proto::init_protocol_context(
                    params.storage_data_dir,
                    params.genesis,
                    params.protocol_overrides,
                    params.commit_genesis,
                    params.enable_testchain,
                    params.readonly,
                    params.patch_context,
                );
                tx.send(&NodeMessage::InitProtocolContextResult(res))?;
            }
            ProtocolMessage::GenesisResultDataCall(params) => {
                let res = Proto::genesis_result_data(
                    &params.genesis_context_hash,
                    &params.chain_id,
                    &params.genesis_protocol_hash,
                    params.genesis_max_operations_ttl,
                );
                tx.send(&NodeMessage::CommitGenesisResultData(res))?;
            }
            ProtocolMessage::GenerateIdentity(params) => {
                let res = Proto::generate_identity(params.expected_pow);
                tx.send(&NodeMessage::GenerateIdentityResult(res))?;
            }
            ProtocolMessage::ShutdownCall => {
                context_send(ContextAction::Shutdown).expect("Failed to send shutdown command to context channel");
                tx.send(&NodeMessage::ShutdownResult)?;
                break;
            }
        }
    }

    Ok(())
}

/// Error types generated by a tezos protocol.
#[derive(Fail, Debug)]
pub enum ProtocolError {
    /// Protocol rejected to apply a block.
    #[fail(display = "Apply block error: {}", reason)]
    ApplyBlockError {
        reason: ApplyBlockError
    },
    #[fail(display = "Begin construction error: {}", reason)]
    BeginConstructionError {
        reason: BeginConstructionError
    },
    #[fail(display = "Validate operation error: {}", reason)]
    ValidateOperationError {
        reason: ValidateOperationError
    },
    #[fail(display = "Protocol rpc call error: {}", reason)]
    ProtocolRpcError {
        reason: ProtocolRpcError
    },
    /// Error in configuration.
    #[fail(display = "OCaml runtime configuration error: {}", reason)]
    TezosRuntimeConfigurationError {
        reason: TezosRuntimeConfigurationError
    },
    /// OCaml part failed to initialize tezos storage.
    #[fail(display = "OCaml storage init error: {}", reason)]
    OcamlStorageInitError {
        reason: TezosStorageInitError
    },
    /// OCaml part failed to generate identity.
    #[fail(display = "Failed to generate tezos identity: {}", reason)]
    TezosGenerateIdentityError {
        reason: TezosGenerateIdentityError
    },
    /// OCaml part failed to get genesis data.
    #[fail(display = "Failed to get genesis data: {}", reason)]
    GenesisResultDataError {
        reason: GetDataError
    },
}

/// Errors generated by `protocol_runner`.
#[derive(Fail, Debug)]
pub enum ProtocolServiceError {
    /// Generic IPC communication error. See `reason` for more details.
    #[fail(display = "IPC error: {}", reason)]
    IpcError {
        reason: IpcError,
    },
    /// Tezos protocol error.
    #[fail(display = "Protocol error: {}", reason)]
    ProtocolError {
        reason: ProtocolError,
    },
    /// Unexpected message was received from IPC channel
    #[fail(display = "Received unexpected message: {}", message)]
    UnexpectedMessage {
        message: &'static str,
    },
    /// Tezedge node failed to spawn new `protocol_runner` sub-process.
    #[fail(display = "Failed to spawn tezos protocol wrapper sub-process: {}", reason)]
    SpawnError {
        reason: io::Error,
    },
    /// Invalid data error
    #[fail(display = "Invalid data error: {}", message)]
    InvalidDataError {
        message: String,
    },
    /// Lock error
    #[fail(display = "Lock error: {:?}", message)]
    LockPoisonError {
        message: String,
    },
}

impl slog::Value for ProtocolServiceError {
    fn serialize(&self, _record: &slog::Record, key: slog::Key, serializer: &mut dyn slog::Serializer) -> slog::Result {
        serializer.emit_arguments(key, &format_args!("{}", self))
    }
}

impl From<IpcError> for ProtocolServiceError {
    fn from(error: IpcError) -> Self {
        ProtocolServiceError::IpcError { reason: error }
    }
}

impl From<ProtocolError> for ProtocolServiceError {
    fn from(error: ProtocolError) -> Self {
        ProtocolServiceError::ProtocolError { reason: error }
    }
}

/// Protocol configuration (transferred via IPC from tezedge node to protocol_runner.
#[derive(Clone, Getters, CopyGetters)]
pub struct ProtocolEndpointConfiguration {
    #[get = "pub"]
    runtime_configuration: TezosRuntimeConfiguration,
    #[get = "pub"]
    environment: TezosEnvironmentConfiguration,
    #[get_copy = "pub"]
    enable_testchain: bool,
    #[get = "pub"]
    data_dir: PathBuf,
    #[get = "pub"]
    executable_path: PathBuf,
    #[get = "pub"]
    log_level: Level,
    #[get_copy = "pub"]
    need_event_server: bool,
}

impl ProtocolEndpointConfiguration {
    pub fn new<P: AsRef<Path>>(runtime_configuration: TezosRuntimeConfiguration, environment: TezosEnvironmentConfiguration, enable_testchain: bool, data_dir: P, executable_path: P, log_level: Level, need_event_server: bool) -> Self {
        ProtocolEndpointConfiguration {
            runtime_configuration,
            environment,
            enable_testchain,
            data_dir: data_dir.as_ref().into(),
            executable_path: executable_path.as_ref().into(),
            log_level,
            need_event_server,
        }
    }
}

/// IPC command server is listening for incoming IPC connections.
pub struct IpcCmdServer(IpcServer<NodeMessage, ProtocolMessage>, ProtocolEndpointConfiguration);

/// Difference between `IpcCmdServer` and `IpcEvtServer` is:
/// * `IpcCmdServer` is used to create IPC channel over which commands from node are transferred to the protocol runner.
/// * `IpcEvtServer` is used to create IPC channel over which events are transmitted from protocol runner to the tezedge node.
impl IpcCmdServer {
    const IO_TIMEOUT: Duration = Duration::from_secs(10);

    /// Create new IPC endpoint
    pub fn new(configuration: ProtocolEndpointConfiguration) -> Self {
        IpcCmdServer(IpcServer::bind_path(&temp_sock()).unwrap(), configuration)
    }

    /// Start accepting incoming IPC connection.
    ///
    /// Returns a [`protocol controller`](ProtocolController) if new IPC channel is successfully created.
    /// This is a blocking operation.
    pub fn accept(&mut self) -> Result<ProtocolController, IpcError> {
        let (rx, tx) = self.0.accept()?;
        // configure IO timeouts
        rx.set_read_timeout(Some(Self::IO_TIMEOUT))
            .and(tx.set_write_timeout(Some(Self::IO_TIMEOUT)))
            .map_err(|err| IpcError::SocketConfigurationError { reason: err })?;

        Ok(ProtocolController {
            io: RefCell::new(IpcIO { rx, tx }),
            configuration: self.1.clone(),
        })
    }
}

/// IPC event server is listening for incoming IPC connections.
pub struct IpcEvtServer(IpcServer<ContextAction, NoopMessage>);

/// Difference between `IpcCmdServer` and `IpcEvtServer` is:
/// * `IpcCmdServer` is used to create IPC channel over which commands from node are transferred to the protocol runner.
/// * `IpcEvtServer` is used to create IPC channel over which events are transmitted from protocol runner to the tezedge node.
impl IpcEvtServer {
    pub fn new() -> Self {
        IpcEvtServer(IpcServer::bind_path(&temp_sock()).unwrap())
    }

    /// Synchronously wait for new incoming IPC connection.
    pub fn accept(&mut self) -> Result<IpcReceiver<ContextAction>, IpcError> {
        let (rx, _) = self.0.accept()?;
        Ok(rx)
    }
}

struct IpcIO {
    rx: IpcReceiver<NodeMessage>,
    tx: IpcSender<ProtocolMessage>,
}

/// Encapsulate IPC communication.
pub struct ProtocolController {
    io: RefCell<IpcIO>,
    configuration: ProtocolEndpointConfiguration,
}

/// Provides convenience methods for IPC communication.
///
/// Instead of manually sending and receiving messages over IPC channel use provided methods.
/// Methods also handle things such as timeouts and also checks is correct response type is received.
impl ProtocolController {
    const GENERATE_IDENTITY_TIMEOUT: Duration = Duration::from_secs(600);
    const APPLY_BLOCK_TIMEOUT: Duration = Duration::from_secs(600);
    const INIT_PROTOCOL_CONTEXT_TIMEOUT: Duration = Duration::from_secs(60);
    const BEGIN_CONSTRUCTION_TIMEOUT: Duration = Duration::from_secs(120);
    const VALIDATE_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
    const CALL_PROTOCOL_RPC_TIMEOUT: Duration = Duration::from_secs(30);

    /// Apply block
    pub fn apply_block(&self, request: ApplyBlockRequest) -> Result<ApplyBlockResponse, ProtocolServiceError> {
        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::ApplyBlockCall(request))?;
        // this might take a while, so we will use unusually long timeout
        io.rx.set_read_timeout(Some(Self::APPLY_BLOCK_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        let receive_result = io.rx.receive();
        // restore default timeout setting
        io.rx.set_read_timeout(Some(IpcCmdServer::IO_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        match receive_result? {
            NodeMessage::ApplyBlockResult(result) => result.map_err(|err| ProtocolError::ApplyBlockError { reason: err }.into()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }

    /// Begin construction
    pub fn begin_construction(&self, request: BeginConstructionRequest) -> Result<PrevalidatorWrapper, ProtocolServiceError> {
        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::BeginConstructionCall(request))?;
        // this might take a while, so we will use unusually long timeout
        io.rx.set_read_timeout(Some(Self::BEGIN_CONSTRUCTION_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        let receive_result = io.rx.receive();
        // restore default timeout setting
        io.rx.set_read_timeout(Some(IpcCmdServer::IO_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        match receive_result? {
            NodeMessage::BeginConstructionResult(result) => result.map_err(|err| ProtocolError::BeginConstructionError { reason: err }.into()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }

    /// Validate operation
    pub fn validate_operation(&self, request: ValidateOperationRequest) -> Result<ValidateOperationResponse, ProtocolServiceError> {
        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::ValidateOperationCall(request))?;
        // this might take a while, so we will use unusually long timeout
        io.rx.set_read_timeout(Some(Self::VALIDATE_OPERATION_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        let receive_result = io.rx.receive();
        // restore default timeout setting
        io.rx.set_read_timeout(Some(IpcCmdServer::IO_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        match receive_result? {
            NodeMessage::ValidateOperationResponse(result) => result.map_err(|err| ProtocolError::ValidateOperationError { reason: err }.into()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }

    /// Call protocol json rpc - internal
    fn call_protocol_json_rpc_internal(&self, msg: ProtocolMessage) -> Result<JsonRpcResponse, ProtocolServiceError> {
        let mut io = self.io.borrow_mut();
        io.tx.send(&msg)?;
        // this might take a while, so we will use unusually long timeout
        io.rx.set_read_timeout(Some(Self::CALL_PROTOCOL_RPC_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        let receive_result = io.rx.receive();
        // restore default timeout setting
        io.rx.set_read_timeout(Some(IpcCmdServer::IO_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        match receive_result? {
            NodeMessage::JsonRpcResponse(result) => result.map_err(|err| ProtocolError::ProtocolRpcError { reason: err }.into()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }

    /// Call protocol json rpc
    pub fn call_protocol_json_rpc(&self, request: ProtocolJsonRpcRequest) -> Result<JsonRpcResponse, ProtocolServiceError> {
        self.call_protocol_json_rpc_internal(ProtocolMessage::ProtocolJsonRpcCall(request))
    }

    /// Call helpers_preapply_operations shell service
    pub fn helpers_preapply_operations(&self, request: ProtocolJsonRpcRequest) -> Result<JsonRpcResponse, ProtocolServiceError> {
        self.call_protocol_json_rpc_internal(ProtocolMessage::HelpersPreapplyOperationsCall(request))
    }

    /// Call helpers_preapply_block shell service
    pub fn helpers_preapply_block(&self, request: ProtocolJsonRpcRequest) -> Result<JsonRpcResponse, ProtocolServiceError> {
        self.call_protocol_json_rpc_internal(ProtocolMessage::HelpersPreapplyBlockCall(request))
    }

    /// Change tezos runtime configuration
    pub fn change_runtime_configuration(&self, settings: TezosRuntimeConfiguration) -> Result<(), ProtocolServiceError> {
        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::ChangeRuntimeConfigurationCall(settings))?;
        match io.rx.receive()? {
            NodeMessage::ChangeRuntimeConfigurationResult(result) => result.map_err(|err| ProtocolError::TezosRuntimeConfigurationError { reason: err }.into()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }

    /// Command tezos ocaml code to initialize context and protocol.
    /// CommitGenesisResult is returned only if commit_genesis is set to true
    fn init_protocol_context(&self,
                             storage_data_dir: String,
                             tezos_environment: &TezosEnvironmentConfiguration,
                             commit_genesis: bool,
                             enable_testchain: bool,
                             readonly: bool,
                             patch_context: Option<PatchContext>) -> Result<InitProtocolContextResult, ProtocolServiceError> {

        // try to check if was at least one write success, other words, if context was already created on file system
        {
            // lock
            let lock: Arc<(Mutex<bool>, Condvar)> = AT_LEAST_ONE_WRITE_PROTOCOL_CONTEXT_WAS_SUCCESS_AT_FIRST_LOCK.clone();
            let &(ref lock, ref cvar) = &*lock;
            let was_one_write_success = lock.lock().map_err(|error| ProtocolServiceError::LockPoisonError { message: format!("{:?}", error) })?;

            // if we have already one write, we can just continue, if not we put thread to sleep and wait
            if !(*was_one_write_success) {
                if readonly {
                    // release lock here and wait
                    let _ = cvar.wait(was_one_write_success).unwrap();
                }
                // TODO: handle situation, thah more writes - we cannot allowed to do so, just one write can exists
            }
        }

        // call init
        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::InitProtocolContextCall(InitProtocolContextParams {
            storage_data_dir,
            genesis: tezos_environment.genesis.clone(),
            genesis_max_operations_ttl: tezos_environment.genesis_additional_data().max_operations_ttl,
            protocol_overrides: tezos_environment.protocol_overrides.clone(),
            commit_genesis,
            enable_testchain,
            readonly,
            patch_context,
        }))?;

        // wait for response
        // this might take a while, so we will use unusually long timeout
        io.rx.set_read_timeout(Some(Self::INIT_PROTOCOL_CONTEXT_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        let receive_result = io.rx.receive();
        // restore default timeout setting
        io.rx.set_read_timeout(Some(IpcCmdServer::IO_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;

        // process response
        match receive_result? {
            NodeMessage::InitProtocolContextResult(result) => {
                if result.is_ok() {
                    // if context is initialized, and is not readonly, means is write, for wich we wait
                    // we check if it is the first one, if it is the first one, we can notify other threads to continue
                    if !readonly {
                        // check if first write success
                        let lock: Arc<(Mutex<bool>, Condvar)> = AT_LEAST_ONE_WRITE_PROTOCOL_CONTEXT_WAS_SUCCESS_AT_FIRST_LOCK.clone();
                        let &(ref lock, ref cvar) = &*lock;
                        let mut was_one_write_success = lock.lock().map_err(|error| ProtocolServiceError::LockPoisonError { message: format!("{:?}", error) })?;
                        if !(*was_one_write_success) {
                            *was_one_write_success = true;
                            cvar.notify_all();
                        }
                    }
                }
                result.map_err(|err| ProtocolError::OcamlStorageInitError { reason: err }.into())
            }
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }

    /// Command tezos ocaml code to generate a new identity.
    pub fn generate_identity(&self, expected_pow: f64) -> Result<Identity, ProtocolServiceError> {
        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::GenerateIdentity(GenerateIdentityParams {
            expected_pow,
        }))?;
        // this might take a while, so we will use unusually long timeout
        io.rx.set_read_timeout(Some(Self::GENERATE_IDENTITY_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        let receive_result = io.rx.receive();
        // restore default timeout setting
        io.rx.set_read_timeout(Some(IpcCmdServer::IO_TIMEOUT)).map_err(|err| IpcError::SocketConfigurationError { reason: err })?;
        match receive_result? {
            NodeMessage::GenerateIdentityResult(result) => result.map_err(|err| ProtocolError::TezosGenerateIdentityError { reason: err }.into()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }

    /// Gracefully shutdown protocol runner
    pub fn shutdown(&self) -> Result<(), ProtocolServiceError> {
        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::ShutdownCall)?;
        match io.rx.receive()? {
            NodeMessage::ShutdownResult => Ok(()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() }),
        }
    }

    /// Initialize protocol environment from default configuration (writeable).
    pub fn init_protocol_for_write(&self, commit_genesis: bool, patch_context: &Option<PatchContext>) -> Result<InitProtocolContextResult, ProtocolServiceError> {
        self.change_runtime_configuration(self.configuration.runtime_configuration().clone())?;
        self.init_protocol_context(
            self.configuration.data_dir().to_str().unwrap().to_string(),
            self.configuration.environment(),
            commit_genesis,
            self.configuration.enable_testchain(),
            false,
            patch_context.clone(),
        )
    }

    /// Initialize protocol environment from default configuration (readonly).
    pub fn init_protocol_for_read(&self) -> Result<InitProtocolContextResult, ProtocolServiceError> {
        self.change_runtime_configuration(self.configuration.runtime_configuration().clone())?;
        self.init_protocol_context(
            self.configuration.data_dir().to_str().unwrap().to_string(),
            self.configuration.environment(),
            false,
            self.configuration.enable_testchain(),
            true,
            None,
        )
    }

    /// Gets data for genesis.
    pub fn genesis_result_data(&self, genesis_context_hash: &ContextHash) -> Result<CommitGenesisResult, ProtocolServiceError> {
        let tezos_environment = self.configuration.environment();
        let main_chain_id = tezos_environment.main_chain_id().map_err(|e| ProtocolServiceError::InvalidDataError { message: format!("{:?}", e) })?;
        let protocol_hash = tezos_environment.genesis_protocol().map_err(|e| ProtocolServiceError::InvalidDataError { message: format!("{:?}", e) })?;

        let mut io = self.io.borrow_mut();
        io.tx.send(&ProtocolMessage::GenesisResultDataCall(GenesisResultDataParams {
            genesis_context_hash: genesis_context_hash.clone(),
            chain_id: main_chain_id,
            genesis_protocol_hash: protocol_hash,
            genesis_max_operations_ttl: tezos_environment.genesis_additional_data().max_operations_ttl,
        }))?;
        match io.rx.receive()? {
            NodeMessage::CommitGenesisResultData(result) => result.map_err(|err| ProtocolError::GenesisResultDataError { reason: err }.into()),
            message => Err(ProtocolServiceError::UnexpectedMessage { message: message.into() })
        }
    }
}

impl Drop for ProtocolController {
    fn drop(&mut self) {
        // try to gracefully shutdown protocol runner
        let _ = self.shutdown();
    }
}

/// Endpoint consists of a protocol runner and IPC communication (command and event channels).
pub struct ProtocolRunnerEndpoint<Runner: ProtocolRunner> {
    pub name: String,
    runner: Runner,
    log: Logger,

    pub commands: IpcCmdServer,
    pub events: Option<IpcEvtServer>,
}

impl<Runner: ProtocolRunner + 'static> ProtocolRunnerEndpoint<Runner> {
    pub fn new(name: &str, configuration: ProtocolEndpointConfiguration, log: Logger) -> ProtocolRunnerEndpoint<Runner> {
        let cmd_server = IpcCmdServer::new(configuration.clone());

        let (evt_server, evt_server_path) = if configuration.need_event_server {
            let evt_server = IpcEvtServer::new();
            let evt_server_path = evt_server.0.path.clone();
            (Some(evt_server), Some(evt_server_path))
        } else {
            (None, None)
        };

        ProtocolRunnerEndpoint {
            name: name.to_string(),
            runner: Runner::new(configuration, cmd_server.0.client().path(), evt_server_path, name.to_string()),
            commands: cmd_server,
            events: evt_server,
            log,
        }
    }

    /// Starts protocol runner sub-process just once and you can take care of it
    pub fn start(&self) -> Result<Runner::Subprocess, ProtocolServiceError> {
        info!(self.log, "Starting protocol runner process"; "endpoint" => self.name.clone());
        self.runner.spawn()
    }

    /// Starts protocol runner sub-process and takes care of it automatically.
    /// If sub-process failed, it is automatically spawned another sub-process.
    /// Returns AtomicBool, if set to false, than terminates sub-process
    pub fn start_in_restarting_mode(&mut self) -> Result<Arc<AtomicBool>, ProtocolServiceError> {
        let run_restarting_feature = Arc::new(AtomicBool::new(true));
        {
            let log = self.log.clone();
            let run = run_restarting_feature.clone();
            let runner = self.runner.clone();
            let name = self.name.clone();
            let mut protocol_runner_process = self.start()?;

            // watchdog thread, which checks if sub-process is running, if not, than starts new one
            thread::spawn(move || {
                while run.load(Ordering::Acquire) {
                    if !Runner::is_running(&mut protocol_runner_process) {
                        info!(log, "Restarting protocol runner process"; "endpoint" => name.clone());
                        protocol_runner_process = match runner.spawn() {
                            Ok(process) => {
                                info!(log, "Protocol runner restarted successfully"; "endpoint" => name.clone());
                                process
                            }
                            Err(e) => {
                                crit!(log, "Failed to spawn protocol runner process"; "endpoint" => name.clone(), "reason" => e);
                                break;
                            }
                        };
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                debug!(log, "Protocol runner stopped restarting_mode"; "endpoint" => name.clone());

                if Runner::is_running(&mut protocol_runner_process) {
                    Runner::terminate(protocol_runner_process);
                }
            })
        };

        Ok(run_restarting_feature)
    }
}

/// Control protocol runner sub-process.
#[derive(Clone)]
pub struct ExecutableProtocolRunner {
    sock_cmd_path: PathBuf,
    sock_evt_path: Option<PathBuf>,
    executable_path: PathBuf,
    endpoint_name: String,
    log_level: Level,
}

impl ExecutableProtocolRunner {
    const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(4);
}

impl ProtocolRunner for ExecutableProtocolRunner {
    type Subprocess = Child;

    fn new(
        configuration: ProtocolEndpointConfiguration,
        sock_cmd_path: &Path,
        sock_evt_path: Option<PathBuf>,
        endpoint_name: String) -> Self {
        ExecutableProtocolRunner {
            sock_cmd_path: sock_cmd_path.to_path_buf(),
            sock_evt_path,
            executable_path: configuration.executable_path.clone(),
            endpoint_name,
            log_level: configuration.log_level,
        }
    }

    fn spawn(&self) -> Result<Self::Subprocess, ProtocolServiceError> {
        let process = match &self.sock_evt_path {
            Some(sep) => Command::new(&self.executable_path)
                .arg("--sock-cmd")
                .arg(&self.sock_cmd_path)
                .arg("--sock-evt")
                .arg(&sep)
                .arg("--endpoint")
                .arg(&self.endpoint_name)
                .arg("--log-level")
                .arg(&self.log_level.as_str().to_lowercase())
                .spawn()
                .map_err(|err| ProtocolServiceError::SpawnError { reason: err })?,
            None => Command::new(&self.executable_path)
                .arg("--sock-cmd")
                .arg(&self.sock_cmd_path)
                .arg("--endpoint")
                .arg(&self.endpoint_name)
                .arg("--log-level")
                .arg(&self.log_level.as_str().to_lowercase())
                .spawn()
                .map_err(|err| ProtocolServiceError::SpawnError { reason: err })?,
        };
        Ok(process)
    }

    fn terminate(mut process: Self::Subprocess) {
        match process.wait_timeout(Self::PROCESS_WAIT_TIMEOUT).unwrap() {
            Some(_) => (),
            None => {
                // child hasn't exited yet
                let _ = process.kill();
            }
        };
    }

    fn terminate_ref(process: &mut Self::Subprocess) {
        match process.wait_timeout(Self::PROCESS_WAIT_TIMEOUT).unwrap() {
            Some(_) => (),
            None => {
                // child hasn't exited yet
                let _ = process.kill();
            }
        };
    }

    fn is_running(process: &mut Self::Subprocess) -> bool {
        match process.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }
}

pub trait ProtocolRunner: Clone + Send + Sync {
    type Subprocess: Send;

    fn new(configuration: ProtocolEndpointConfiguration, sock_cmd_path: &Path, sock_evt_path: Option<PathBuf>, endpoint_name: String) -> Self;

    fn spawn(&self) -> Result<Self::Subprocess, ProtocolServiceError>;

    fn terminate(process: Self::Subprocess);
    fn terminate_ref(process: &mut Self::Subprocess);

    fn is_running(process: &mut Self::Subprocess) -> bool;
}
