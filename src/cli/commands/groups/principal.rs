// EasyNet CLI — PrincipalLifecycle Group
// =====================================
//
// File: src/cli/commands/groups/principal.rs
// Description: Product-neutral CLI facade for daemon-owned
//              `principal.lifecycle.*` runtime abilities.
//
// Protocol Responsibility:
// - Lowers operator CLI arguments into PrincipalLifecycle ability payloads.
// - Preserves explicit command/proof/idempotency fields instead of inventing
//   a second authentication model.
//
// Implementation Approach:
// - Dispatches through the shared local ability invocation helper.
// - Uses daemon key-service public projections for bind-first-key, never
//   reading or returning private key material.
//
// Usage Contract:
// - This is standalone-Hub/runtime administration, not Backend login.
// - Product account/OAuth/HTTP concepts must stay out of this command group.
//
// Architectural Position:
// - CLI facade over EasyNet-Cli daemon PrincipalLifecycle aggregate.

use anyhow::{anyhow, Context};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{json, Value};
use uuid::Uuid;

use super::principal_routes_gen as routes;
use crate::core::ura;
use crate::daemon::identity::self_identity::KeyringClient;
use crate::daemon::keyring::{ManagedSigningKeyProjection, ManagedSigningStatus};
use crate::support::platform::local_invoke::{LocalAbilityTarget, LocalDaemonSystemAbilityIssuer};
use crate::support::platform::output;

const PROFILE_PURPOSE: &str = "user_signing.cli";

#[derive(Debug, Args)]
pub struct PrincipalArgs {
    #[command(subcommand)]
    pub action: PrincipalAction,
}

#[derive(Debug, Subcommand)]
pub enum PrincipalAction {
    /// Create the first Principal and bind its first daemon-managed key.
    Bootstrap(BootstrapArgs),
    /// Consume an enrollment capability and bind the Principal's first key.
    Enroll(EnrollArgs),
    /// Create a pending Principal URA through the daemon lifecycle aggregate.
    Create(CreateArgs),
    /// Issue a one-time enrollment capability for another Principal URA.
    IssueEnrollment(IssueEnrollmentArgs),
    /// Revoke an unconsumed enrollment capability.
    RevokeEnrollment(RevokeEnrollmentArgs),
    /// Create or reuse a daemon-managed signing key and bind it as first key.
    BindFirstKey(BindFirstKeyArgs),
    /// Add another public-key binding to an active Principal.
    AddKey(KeyMutationArgs),
    /// Rotate one active binding to a replacement public key.
    RotateKey(RotateKeyArgs),
    /// Revoke one active public-key binding.
    RevokeKey(BindingMutationArgs),
    /// Configure the recovery policy reference for a Principal.
    ConfigureRecovery(ConfigureRecoveryArgs),
    /// Use a recovery proof to add a replacement key and reactivate a Principal.
    Recover(RecoverArgs),
    /// Suspend an active Principal.
    Suspend(StateMutationArgs),
    /// Reactivate a suspended Principal.
    Reactivate(StateMutationArgs),
    /// Delete a Principal. Deleted state is terminal.
    Delete(StateMutationArgs),
    /// Issue a revocable lifecycle grant for the Principal.
    IssueGrant(IssueGrantArgs),
    /// Revoke a lifecycle grant for the Principal.
    RevokeGrant(RevokeGrantArgs),
    /// Read one PrincipalLifecycle public snapshot.
    Get(GetArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProofKindArg {
    Bootstrap,
    ActiveKey,
    Grant,
    Enrollment,
    Recovery,
}

impl ProofKindArg {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::ActiveKey => "active_key",
            Self::Grant => "grant",
            Self::Enrollment => "enrollment",
            Self::Recovery => "recovery",
        }
    }
}

#[derive(Debug, Args)]
pub struct BootstrapArgs {
    #[arg(long)]
    pub principal_ura: String,
    /// Optional one-time bootstrap proof reference shared by create and bind.
    #[arg(long)]
    pub proof_ref: Option<String>,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub create_idempotency_key: Option<String>,
    #[arg(long)]
    pub bind_idempotency_key: Option<String>,
    #[arg(long)]
    pub key_id: Option<String>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub show_pubkey: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EnrollArgs {
    #[arg(long)]
    pub principal_ura: String,
    /// Enrollment capability id issued for this Principal URA.
    #[arg(long)]
    pub enrollment_id: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub create_idempotency_key: Option<String>,
    #[arg(long)]
    pub bind_idempotency_key: Option<String>,
    #[arg(long)]
    pub key_id: Option<String>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub show_pubkey: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::Bootstrap)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: Option<String>,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct IssueEnrollmentArgs {
    /// Existing active principal that owns the enrollment authority.
    #[arg(long)]
    pub issuer_ura: String,
    /// Principal URA that the enrollment capability may create.
    #[arg(long)]
    pub subject_principal_ura: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    /// Active key binding id or grant id authorizing this transition.
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RevokeEnrollmentArgs {
    #[arg(long)]
    pub issuer_ura: String,
    #[arg(long)]
    pub enrollment_id: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BindFirstKeyArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::Bootstrap)]
    pub proof_kind: ProofKindArg,
    /// Must match the proof used by `principal create`.
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub key_id: Option<String>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub show_pubkey: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct KeyMutationArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    /// Explicit public-key projection to bind instead of creating a local key.
    #[arg(long)]
    pub public_key_b64: Option<String>,
    /// Public key identifier to store with an explicit public key projection.
    #[arg(long)]
    pub key_id: Option<String>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub show_pubkey: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RecoverArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::Recovery)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    /// Explicit public-key projection to bind instead of creating a local key.
    #[arg(long)]
    pub public_key_b64: Option<String>,
    /// Public key identifier to store with an explicit public key projection.
    #[arg(long)]
    pub key_id: Option<String>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub show_pubkey: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RotateKeyArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long)]
    pub binding_id: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    /// Existing daemon-managed key to rotate through key-service.
    #[arg(long)]
    pub rotate_from_key_id: Option<String>,
    /// Explicit replacement public key projection.
    #[arg(long)]
    pub replacement_public_key_b64: Option<String>,
    /// Public key identifier to store with an explicit replacement key.
    #[arg(long)]
    pub replacement_key_id: Option<String>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub show_pubkey: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BindingMutationArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long)]
    pub binding_id: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ConfigureRecoveryArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long)]
    pub policy_ref: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StateMutationArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long, value_enum)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct IssueGrantArgs {
    #[arg(long)]
    pub principal_ura: String,
    /// Grant action. Repeat for multiple lifecycle actions.
    #[arg(long = "action", required = true)]
    pub actions: Vec<String>,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub expires_unix_ms: Option<i64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RevokeGrantArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long)]
    pub grant_id: String,
    #[arg(long, value_enum, default_value_t = ProofKindArg::ActiveKey)]
    pub proof_kind: ProofKindArg,
    #[arg(long)]
    pub proof_ref: String,
    #[arg(long)]
    pub actor_ura: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long)]
    pub expected_version: Option<u64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct GetArgs {
    #[arg(long)]
    pub principal_ura: String,
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: PrincipalArgs) -> anyhow::Result<()> {
    match args.action {
        PrincipalAction::Bootstrap(args) => run_bootstrap(args),
        PrincipalAction::Enroll(args) => run_enroll(args),
        PrincipalAction::Create(args) => run_create(args),
        PrincipalAction::IssueEnrollment(args) => run_issue_enrollment(args),
        PrincipalAction::RevokeEnrollment(args) => run_revoke_enrollment(args),
        PrincipalAction::BindFirstKey(args) => run_bind_first_key(args),
        PrincipalAction::AddKey(args) => run_add_key(args),
        PrincipalAction::RotateKey(args) => run_rotate_key(args),
        PrincipalAction::RevokeKey(args) => run_revoke_key(args),
        PrincipalAction::ConfigureRecovery(args) => run_configure_recovery(args),
        PrincipalAction::Recover(args) => run_recover(args),
        PrincipalAction::Suspend(args) => run_state_mutation(
            routes::PRINCIPAL_ABILITY_SUSPEND,
            "Suspended principal",
            args,
        ),
        PrincipalAction::Reactivate(args) => run_state_mutation(
            routes::PRINCIPAL_ABILITY_REACTIVATE,
            "Reactivated principal",
            args,
        ),
        PrincipalAction::Delete(args) => {
            run_state_mutation(routes::PRINCIPAL_ABILITY_DELETE, "Deleted principal", args)
        }
        PrincipalAction::IssueGrant(args) => run_issue_grant(args),
        PrincipalAction::RevokeGrant(args) => run_revoke_grant(args),
        PrincipalAction::Get(args) => run_get(args),
    }
}

fn run_bootstrap(args: BootstrapArgs) -> anyhow::Result<()> {
    let key = ensure_principal_signing_key(&KeyringClient::default_path(), &args.principal_ura)?;
    let create_idempotency_key =
        command_id_with_prefix(args.create_idempotency_key, "principal-bootstrap-create");
    let proof_ref = bootstrap_proof_ref(args.proof_ref, &create_idempotency_key);
    let bind_idempotency_key =
        command_id_with_prefix(args.bind_idempotency_key, "principal-bootstrap-bind");
    let input = FirstKeyRequestInput {
        principal_ura: &args.principal_ura,
        actor_ura: args.actor_ura.as_deref(),
        create_idempotency_key: &create_idempotency_key,
        bind_idempotency_key: &bind_idempotency_key,
        key_id: args.key_id.as_deref().unwrap_or(key.key_id.as_str()),
        public_key_b64: &key.public_key_b64,
        expires_unix_ms: args.expires_unix_ms,
    };
    let (create_request, bind_request) = principal_bootstrap_requests(input, &proof_ref);

    invoke_principal_ability(routes::PRINCIPAL_ABILITY_CREATE, create_request)?;
    let response =
        invoke_principal_ability(routes::PRINCIPAL_ABILITY_BIND_FIRST_KEY, bind_request)?;
    render_snapshot(
        "Bootstrapped principal",
        response,
        args.json,
        if args.show_pubkey {
            Some(format!("public_key_b64={}", key.public_key_b64))
        } else {
            None
        },
    )
}

fn run_enroll(args: EnrollArgs) -> anyhow::Result<()> {
    let key = ensure_principal_signing_key(&KeyringClient::default_path(), &args.principal_ura)?;
    let create_idempotency_key =
        command_id_with_prefix(args.create_idempotency_key, "principal-enroll-create");
    let bind_idempotency_key =
        command_id_with_prefix(args.bind_idempotency_key, "principal-enroll-bind");
    let input = FirstKeyRequestInput {
        principal_ura: &args.principal_ura,
        actor_ura: args.actor_ura.as_deref(),
        create_idempotency_key: &create_idempotency_key,
        bind_idempotency_key: &bind_idempotency_key,
        key_id: args.key_id.as_deref().unwrap_or(key.key_id.as_str()),
        public_key_b64: &key.public_key_b64,
        expires_unix_ms: args.expires_unix_ms,
    };
    let (create_request, bind_request) = principal_enrollment_requests(input, &args.enrollment_id);

    invoke_principal_ability(routes::PRINCIPAL_ABILITY_CREATE, create_request)?;
    let response =
        invoke_principal_ability(routes::PRINCIPAL_ABILITY_BIND_FIRST_KEY, bind_request)?;
    render_snapshot(
        "Enrolled principal",
        response,
        args.json,
        if args.show_pubkey {
            Some(format!("public_key_b64={}", key.public_key_b64))
        } else {
            None
        },
    )
}

fn run_create(args: CreateArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let proof_ref = args
        .proof_ref
        .unwrap_or_else(|| default_proof_ref(&idempotency_key));
    let request = principal_create_request(
        &args.principal_ura,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &proof_ref,
        ),
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_CREATE, request)?;
    render_snapshot("Created principal", response, args.json, None)
}

fn run_issue_enrollment(args: IssueEnrollmentArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_issue_enrollment_request(
        &args.issuer_ura,
        &args.subject_principal_ura,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.issuer_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
        args.expires_unix_ms,
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_ISSUE_ENROLLMENT, request)?;
    let extra = newest_enrollment_id(&response);
    render_snapshot("Issued enrollment", response, args.json, Some(extra))
}

fn run_revoke_enrollment(args: RevokeEnrollmentArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_revoke_enrollment_request(
        &args.issuer_ura,
        &args.enrollment_id,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.issuer_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_REVOKE_ENROLLMENT, request)?;
    render_snapshot("Revoked enrollment", response, args.json, None)
}

fn run_bind_first_key(args: BindFirstKeyArgs) -> anyhow::Result<()> {
    let key = ensure_principal_signing_key(&KeyringClient::default_path(), &args.principal_ura)?;
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_bind_key_request(
        &args.principal_ura,
        key.public_key_b64.as_str(),
        args.key_id.as_deref().unwrap_or(key.key_id.as_str()),
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
        args.expires_unix_ms,
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_BIND_FIRST_KEY, request)?;
    render_snapshot(
        "Bound first key",
        response,
        args.json,
        if args.show_pubkey {
            Some(format!("public_key_b64={}", key.public_key_b64))
        } else {
            None
        },
    )
}

fn run_add_key(args: KeyMutationArgs) -> anyhow::Result<()> {
    let key = resolve_principal_signing_key(
        &KeyringClient::default_path(),
        &args.principal_ura,
        KeySource {
            public_key_b64: args.public_key_b64.as_deref(),
            key_id: args.key_id.as_deref(),
            rotate_from_key_id: None,
        },
    )?;
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_bind_key_request(
        &args.principal_ura,
        key.public_key_b64.as_str(),
        key.key_id.as_str(),
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
        args.expires_unix_ms,
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_ADD_KEY, request)?;
    render_snapshot(
        "Added key",
        response,
        args.json,
        public_key_extra(args.show_pubkey, &key),
    )
}

fn run_rotate_key(args: RotateKeyArgs) -> anyhow::Result<()> {
    let key = resolve_principal_signing_key(
        &KeyringClient::default_path(),
        &args.principal_ura,
        KeySource {
            public_key_b64: args.replacement_public_key_b64.as_deref(),
            key_id: args.replacement_key_id.as_deref(),
            rotate_from_key_id: args.rotate_from_key_id.as_deref(),
        },
    )?;
    let idempotency_key = command_id(args.idempotency_key);
    let command = principal_command(
        PrincipalCommandActor::supplied_or_subject_self(
            args.actor_ura.as_deref(),
            &args.principal_ura,
        ),
        &idempotency_key,
        args.expected_version,
        args.proof_kind,
        &args.proof_ref,
    );
    let request = principal_rotate_key_request(
        &args.principal_ura,
        &args.binding_id,
        key.public_key_b64.as_str(),
        key.key_id.as_str(),
        command,
        args.expires_unix_ms,
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_ROTATE_KEY, request)?;
    render_snapshot(
        "Rotated key",
        response,
        args.json,
        public_key_extra(args.show_pubkey, &key),
    )
}

fn run_revoke_key(args: BindingMutationArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_revoke_key_request(
        &args.principal_ura,
        &args.binding_id,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_REVOKE_KEY, request)?;
    render_snapshot("Revoked key", response, args.json, None)
}

fn run_configure_recovery(args: ConfigureRecoveryArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_configure_recovery_request(
        &args.principal_ura,
        &args.policy_ref,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_CONFIGURE_RECOVERY, request)?;
    render_snapshot("Configured recovery", response, args.json, None)
}

fn run_recover(args: RecoverArgs) -> anyhow::Result<()> {
    let key = resolve_principal_signing_key(
        &KeyringClient::default_path(),
        &args.principal_ura,
        KeySource {
            public_key_b64: args.public_key_b64.as_deref(),
            key_id: args.key_id.as_deref(),
            rotate_from_key_id: None,
        },
    )?;
    let idempotency_key = command_id(args.idempotency_key);
    let command = principal_command(
        PrincipalCommandActor::supplied_or_subject_self(
            args.actor_ura.as_deref(),
            &args.principal_ura,
        ),
        &idempotency_key,
        args.expected_version,
        args.proof_kind,
        &args.proof_ref,
    );
    let request = principal_recover_request(
        &args.principal_ura,
        key.public_key_b64.as_str(),
        key.key_id.as_str(),
        command,
        args.expires_unix_ms,
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_RECOVER, request)?;
    render_snapshot(
        "Recovered principal",
        response,
        args.json,
        public_key_extra(args.show_pubkey, &key),
    )
}

fn run_state_mutation(ability: &str, label: &str, args: StateMutationArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_state_request(
        &args.principal_ura,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
    );
    let response = invoke_principal_ability(ability, request)?;
    render_snapshot(label, response, args.json, None)
}

fn run_issue_grant(args: IssueGrantArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_issue_grant_request(
        &args.principal_ura,
        args.actions.as_slice(),
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
        args.expires_unix_ms,
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_ISSUE_GRANT, request)?;
    let extra = newest_grant_id(&response);
    render_snapshot("Issued grant", response, args.json, Some(extra))
}

fn run_revoke_grant(args: RevokeGrantArgs) -> anyhow::Result<()> {
    let idempotency_key = command_id(args.idempotency_key);
    let request = principal_revoke_grant_request(
        &args.principal_ura,
        &args.grant_id,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(
                args.actor_ura.as_deref(),
                &args.principal_ura,
            ),
            &idempotency_key,
            args.expected_version,
            args.proof_kind,
            &args.proof_ref,
        ),
    );
    let response = invoke_principal_ability(routes::PRINCIPAL_ABILITY_REVOKE_GRANT, request)?;
    render_snapshot("Revoked grant", response, args.json, None)
}

fn run_get(args: GetArgs) -> anyhow::Result<()> {
    let response = invoke_principal_ability(
        routes::PRINCIPAL_ABILITY_GET,
        json!({ "principal_ura": args.principal_ura.trim() }),
    )?;
    render_snapshot("Principal snapshot", response, args.json, None)
}

fn invoke_principal_ability(ability: &str, args: Value) -> anyhow::Result<Value> {
    let target = principal_ability_target(ability, &args)?;
    LocalDaemonSystemAbilityIssuer::invoke_target_root_timeout(
        &target,
        args,
        None,
        std::time::Duration::from_secs(30),
    )
    .with_context(|| format!("invoke {ability}"))
}

fn principal_ability_target(ability: &str, args: &Value) -> anyhow::Result<LocalAbilityTarget> {
    let principal_ura = principal_ability_realm_source(args)?;
    let parsed = ura::parse_ura(principal_ura)
        .with_context(|| format!("parse PrincipalLifecycle principal URA {principal_ura:?}"))?;
    if parsed.kind != ura::URAKind::User {
        anyhow::bail!("principal.lifecycle principal_ura must be a User URA");
    }
    let hub_ura = ura::hub_ura(&parsed.realm);
    let ability_ura = ura::owner_ability_ura(&hub_ura, ability)
        .ok_or_else(|| anyhow!("derive Hub PrincipalLifecycle Ability URA for {ability}"))?;
    let selector = ura::AbilitySelector::parse(&ability_ura)?;
    Ok(LocalAbilityTarget::from_selector(&selector))
}

fn principal_ability_realm_source(args: &Value) -> anyhow::Result<&str> {
    args.pointer("/request/principal_ura")
        .or_else(|| args.get("principal_ura"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("principal.lifecycle request missing principal_ura"))
}

#[derive(Clone, Copy)]
enum PrincipalCommandActor<'a> {
    Supplied(&'a str),
    SubjectSelf(&'a str),
}

impl<'a> PrincipalCommandActor<'a> {
    fn supplied_or_subject_self(actor_ura: Option<&'a str>, principal_ura: &'a str) -> Self {
        actor_ura
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Self::Supplied)
            .unwrap_or(Self::SubjectSelf(principal_ura))
    }

    fn subject_self(principal_ura: &'a str) -> Self {
        Self::SubjectSelf(principal_ura)
    }

    fn actor_ura(self) -> &'a str {
        match self {
            Self::Supplied(actor_ura) | Self::SubjectSelf(actor_ura) => actor_ura.trim(),
        }
    }
}

fn principal_command(
    actor: PrincipalCommandActor<'_>,
    idempotency_key: &str,
    expected_version: Option<u64>,
    proof_kind: ProofKindArg,
    proof_ref: &str,
) -> Value {
    let mut command = json!({
        "actor_ura": actor.actor_ura(),
        "idempotency_key": idempotency_key,
        "proof": {
            "kind": proof_kind.as_wire(),
            "reference": proof_ref.trim(),
        }
    });
    if let Some(version) = expected_version {
        command["expected_version"] = json!(version);
    }
    command
}

fn principal_create_request(principal_ura: &str, command: Value) -> Value {
    json!({
        "request": {
            "command": command,
            "principal_ura": principal_ura.trim(),
        }
    })
}

#[derive(Clone, Copy)]
struct FirstKeyRequestInput<'a> {
    principal_ura: &'a str,
    actor_ura: Option<&'a str>,
    create_idempotency_key: &'a str,
    bind_idempotency_key: &'a str,
    key_id: &'a str,
    public_key_b64: &'a str,
    expires_unix_ms: Option<i64>,
}

fn principal_bootstrap_requests(
    input: FirstKeyRequestInput<'_>,
    proof_ref: &str,
) -> (Value, Value) {
    principal_create_and_bind_first_key_requests(input, ProofKindArg::Bootstrap, proof_ref)
}

fn principal_enrollment_requests(
    input: FirstKeyRequestInput<'_>,
    enrollment_id: &str,
) -> (Value, Value) {
    principal_create_and_bind_first_key_requests(input, ProofKindArg::Enrollment, enrollment_id)
}

fn principal_create_and_bind_first_key_requests(
    input: FirstKeyRequestInput<'_>,
    proof_kind: ProofKindArg,
    proof_ref: &str,
) -> (Value, Value) {
    let create_request = principal_create_request(
        input.principal_ura,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(input.actor_ura, input.principal_ura),
            input.create_idempotency_key,
            None,
            proof_kind,
            proof_ref,
        ),
    );
    let bind_request = principal_bind_key_request(
        input.principal_ura,
        input.public_key_b64,
        input.key_id,
        principal_command(
            PrincipalCommandActor::supplied_or_subject_self(input.actor_ura, input.principal_ura),
            input.bind_idempotency_key,
            Some(1),
            proof_kind,
            proof_ref,
        ),
        input.expires_unix_ms,
    );
    (create_request, bind_request)
}

fn principal_issue_enrollment_request(
    issuer_ura: &str,
    subject_principal_ura: &str,
    command: Value,
    expires_unix_ms: Option<i64>,
) -> Value {
    let mut request = json!({
        "command": command,
        "principal_ura": issuer_ura.trim(),
        "subject_principal_ura": subject_principal_ura.trim(),
    });
    if let Some(expires) = expires_unix_ms {
        request["expires_unix_ms"] = json!(expires);
    }
    json!({ "request": request })
}

fn principal_revoke_enrollment_request(
    issuer_ura: &str,
    enrollment_id: &str,
    command: Value,
) -> Value {
    json!({
        "request": {
            "command": command,
            "principal_ura": issuer_ura.trim(),
            "enrollment_id": enrollment_id.trim(),
        }
    })
}

fn principal_bind_key_request(
    principal_ura: &str,
    public_key_b64: &str,
    key_id: &str,
    command: Value,
    expires_unix_ms: Option<i64>,
) -> Value {
    let mut request = json!({
        "command": command,
        "principal_ura": principal_ura.trim(),
        "public_key_b64": public_key_b64.trim(),
    });
    if !key_id.trim().is_empty() {
        request["key_id"] = json!(key_id.trim());
    }
    if let Some(expires) = expires_unix_ms {
        request["expires_unix_ms"] = json!(expires);
    }
    json!({ "request": request })
}

fn principal_rotate_key_request(
    principal_ura: &str,
    binding_id: &str,
    public_key_b64: &str,
    key_id: &str,
    command: Value,
    expires_unix_ms: Option<i64>,
) -> Value {
    let replacement = principal_bind_key_payload(
        principal_ura,
        public_key_b64,
        key_id,
        principal_command(
            PrincipalCommandActor::subject_self(principal_ura),
            "replacement",
            None,
            ProofKindArg::ActiveKey,
            "replacement",
        ),
        expires_unix_ms,
    );
    json!({
        "request": {
            "command": command,
            "principal_ura": principal_ura.trim(),
            "binding_id": binding_id.trim(),
            "replacement": replacement,
        }
    })
}

fn principal_revoke_key_request(principal_ura: &str, binding_id: &str, command: Value) -> Value {
    json!({
        "request": {
            "command": command,
            "principal_ura": principal_ura.trim(),
            "binding_id": binding_id.trim(),
        }
    })
}

fn principal_configure_recovery_request(
    principal_ura: &str,
    policy_ref: &str,
    command: Value,
) -> Value {
    json!({
        "request": {
            "command": command,
            "principal_ura": principal_ura.trim(),
            "policy_ref": policy_ref.trim(),
        }
    })
}

fn principal_recover_request(
    principal_ura: &str,
    public_key_b64: &str,
    key_id: &str,
    command: Value,
    expires_unix_ms: Option<i64>,
) -> Value {
    let replacement_key = principal_bind_key_payload(
        principal_ura,
        public_key_b64,
        key_id,
        principal_command(
            PrincipalCommandActor::subject_self(principal_ura),
            "replacement",
            None,
            ProofKindArg::Recovery,
            "replacement",
        ),
        expires_unix_ms,
    );
    json!({
        "request": {
            "command": command,
            "principal_ura": principal_ura.trim(),
            "replacement_key": replacement_key,
        }
    })
}

fn principal_state_request(principal_ura: &str, command: Value) -> Value {
    json!({
        "request": {
            "command": command,
            "principal_ura": principal_ura.trim(),
        }
    })
}

fn principal_issue_grant_request(
    principal_ura: &str,
    actions: &[String],
    command: Value,
    expires_unix_ms: Option<i64>,
) -> Value {
    let mut request = json!({
        "command": command,
        "principal_ura": principal_ura.trim(),
        "actions": actions
            .iter()
            .map(|action| action.trim())
            .filter(|action| !action.is_empty())
            .collect::<Vec<_>>(),
    });
    if let Some(expires) = expires_unix_ms {
        request["expires_unix_ms"] = json!(expires);
    }
    json!({ "request": request })
}

fn principal_revoke_grant_request(principal_ura: &str, grant_id: &str, command: Value) -> Value {
    json!({
        "request": {
            "command": command,
            "principal_ura": principal_ura.trim(),
            "grant_id": grant_id.trim(),
        }
    })
}

fn principal_bind_key_payload(
    principal_ura: &str,
    public_key_b64: &str,
    key_id: &str,
    command: Value,
    expires_unix_ms: Option<i64>,
) -> Value {
    let mut payload = json!({
        "command": command,
        "principal_ura": principal_ura.trim(),
        "public_key_b64": public_key_b64.trim(),
    });
    if !key_id.trim().is_empty() {
        payload["key_id"] = json!(key_id.trim());
    }
    if let Some(expires) = expires_unix_ms {
        payload["expires_unix_ms"] = json!(expires);
    }
    payload
}

fn command_id(explicit: Option<String>) -> String {
    command_id_with_prefix(explicit, "principal")
}

fn command_id_with_prefix(explicit: Option<String>, prefix: &str) -> String {
    explicit
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{prefix}-{}", Uuid::new_v4().simple()))
}

fn default_proof_ref(idempotency_key: &str) -> String {
    format!("proof:{idempotency_key}")
}

fn bootstrap_proof_ref(explicit: Option<String>, create_idempotency_key: &str) -> String {
    explicit
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_proof_ref(create_idempotency_key))
}

#[derive(Debug, Clone)]
struct PrincipalSigningKey {
    key_id: String,
    public_key_b64: String,
}

trait PrincipalSigningKeyStore {
    fn list(&self) -> anyhow::Result<Vec<ManagedSigningKeyProjection>>;
    fn create(&self, principal_ura: &str) -> anyhow::Result<ManagedSigningKeyProjection>;
    fn rotate(&self, key_id: &str) -> anyhow::Result<ManagedSigningKeyProjection>;
}

impl PrincipalSigningKeyStore for KeyringClient {
    fn list(&self) -> anyhow::Result<Vec<ManagedSigningKeyProjection>> {
        Ok(self.inventory_list(
            Some(PROFILE_PURPOSE.to_string()),
            Some(ManagedSigningStatus::Active),
        )?)
    }

    fn create(&self, principal_ura: &str) -> anyhow::Result<ManagedSigningKeyProjection> {
        Ok(self.inventory_create(PROFILE_PURPOSE, Some(principal_ura.to_string()))?)
    }

    fn rotate(&self, key_id: &str) -> anyhow::Result<ManagedSigningKeyProjection> {
        Ok(self.inventory_rotate(key_id)?)
    }
}

fn ensure_principal_signing_key(
    store: &impl PrincipalSigningKeyStore,
    principal_ura: &str,
) -> anyhow::Result<PrincipalSigningKey> {
    if ura::parse_ura(principal_ura)?.kind != ura::URAKind::User {
        anyhow::bail!("principal_ura must be a User URA");
    }
    if let Some(existing) = store
        .list()?
        .into_iter()
        .find(|entry| entry.bound_subject.as_deref() == Some(principal_ura))
    {
        return Ok(PrincipalSigningKey {
            key_id: existing.key_id,
            public_key_b64: existing.public_key_b64,
        });
    }
    let created = store.create(principal_ura)?;
    Ok(PrincipalSigningKey {
        key_id: created.key_id,
        public_key_b64: created.public_key_b64,
    })
}

struct KeySource<'a> {
    public_key_b64: Option<&'a str>,
    key_id: Option<&'a str>,
    rotate_from_key_id: Option<&'a str>,
}

fn resolve_principal_signing_key(
    store: &impl PrincipalSigningKeyStore,
    principal_ura: &str,
    source: KeySource<'_>,
) -> anyhow::Result<PrincipalSigningKey> {
    if ura::parse_ura(principal_ura)?.kind != ura::URAKind::User {
        anyhow::bail!("principal_ura must be a User URA");
    }
    if source.public_key_b64.is_some() && source.rotate_from_key_id.is_some() {
        anyhow::bail!("public_key_b64 and rotate_from_key_id are mutually exclusive");
    }
    if let Some(public_key_b64) = source
        .public_key_b64
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(PrincipalSigningKey {
            key_id: source.key_id.unwrap_or_default().trim().to_string(),
            public_key_b64: public_key_b64.to_string(),
        });
    }
    if let Some(key_id) = source
        .rotate_from_key_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let rotated = store.rotate(key_id)?;
        return Ok(PrincipalSigningKey {
            key_id: rotated.key_id,
            public_key_b64: rotated.public_key_b64,
        });
    }
    let created = store.create(principal_ura)?;
    Ok(PrincipalSigningKey {
        key_id: created.key_id,
        public_key_b64: created.public_key_b64,
    })
}

fn public_key_extra(show: bool, key: &PrincipalSigningKey) -> Option<String> {
    show.then(|| format!("public_key_b64={}", key.public_key_b64))
}

fn render_snapshot(
    label: &str,
    response: Value,
    as_json: bool,
    extra: Option<String>,
) -> anyhow::Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }
    let principal = response
        .get("principal")
        .ok_or_else(|| anyhow!("principal.lifecycle response did not include principal"))?;
    output::success(label);
    let principal_ura = text_field(principal, "principal_ura");
    let state = text_field(principal, "state");
    let version = text_field(principal, "version");
    output::kv_section_stdout(&[
        ("principal_ura", principal_ura.as_str()),
        ("state", state.as_str()),
        ("version", version.as_str()),
    ]);
    if let Some(extra) = extra {
        println!("  {extra}");
    }
    Ok(())
}

fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| value.get(key).map(|value| value.to_string()))
        .unwrap_or_default()
}

fn newest_enrollment_id(response: &Value) -> String {
    response
        .get("principal")
        .unwrap_or(response)
        .get("enrollments")
        .and_then(Value::as_array)
        .and_then(|items| items.last())
        .and_then(|item| item.get("enrollment_id"))
        .and_then(Value::as_str)
        .map(|id| format!("enrollment_id={id}"))
        .unwrap_or_else(|| "enrollment_id=<not returned>".to_string())
}

fn newest_grant_id(response: &Value) -> String {
    response
        .get("principal")
        .unwrap_or(response)
        .get("grants")
        .and_then(Value::as_array)
        .and_then(|items| items.last())
        .and_then(|item| item.get("grant_id"))
        .and_then(Value::as_str)
        .map(|id| format!("grant_id={id}"))
        .unwrap_or_else(|| "grant_id=<not returned>".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use sha2::Digest as _;
    use std::cell::{Cell, RefCell};
    use std::path::Path;

    #[test]
    fn principal_routes_are_generated_from_manifest() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("provider_routes/easynet-principal-lifecycle-routes.v1.json");
        let digest = sha2::Sha256::digest(std::fs::read(manifest).expect("read manifest"));

        assert_eq!(routes::PRINCIPAL_ROUTE_MANIFEST_SHA256, hex::encode(digest));
        assert_eq!(routes::PRINCIPAL_LIFECYCLE_PROFILE, "principal_lifecycle");
    }

    #[test]
    fn create_request_keeps_explicit_command_boundary() {
        let command = principal_command(
            PrincipalCommandActor::subject_self(" easynet:///r/realm/user/alice "),
            "idem-1",
            Some(1),
            ProofKindArg::Bootstrap,
            "proof-1",
        );
        let request = principal_create_request(" easynet:///r/realm/user/alice ", command);

        assert_eq!(
            request["request"]["principal_ura"],
            "easynet:///r/realm/user/alice"
        );
        assert_eq!(
            request["request"]["command"]["actor_ura"],
            "easynet:///r/realm/user/alice"
        );
        assert_eq!(request["request"]["command"]["proof"]["kind"], "bootstrap");
        assert_eq!(
            request["request"]["command"]["proof"]["reference"],
            "proof-1"
        );
        assert_eq!(request["request"]["command"]["expected_version"], 1);
    }

    #[test]
    fn principal_ability_target_uses_hub_owner_from_principal_realm() {
        let command = principal_command(
            PrincipalCommandActor::subject_self("easynet:///r/realm/user/alice"),
            "idem-1",
            Some(1),
            ProofKindArg::Bootstrap,
            "proof-1",
        );
        let request = principal_create_request("easynet:///r/realm/user/alice", command);
        let target = principal_ability_target(routes::PRINCIPAL_ABILITY_CREATE, &request)
            .expect("principal target");

        assert_eq!(target.dispatch_name(), routes::PRINCIPAL_ABILITY_CREATE);
        assert_eq!(target.callee_ura(), "easynet:///r/realm/authority");
        assert_eq!(
            target.default_subject_ura(),
            "easynet:///r/realm/ability/authority.principal.lifecycle.create"
        );
    }

    #[test]
    fn bootstrap_requests_share_proof_and_lock_bind_version() {
        let (create_request, bind_request) = principal_bootstrap_requests(
            FirstKeyRequestInput {
                principal_ura: " easynet:///r/realm/user/alice ",
                actor_ura: Some(" easynet:///r/realm/user/admin "),
                create_idempotency_key: "create-idem",
                bind_idempotency_key: "bind-idem",
                key_id: "key-1",
                public_key_b64: "PUB",
                expires_unix_ms: Some(12345),
            },
            " bootstrap-proof-1 ",
        );

        assert_eq!(
            create_request["request"]["principal_ura"],
            "easynet:///r/realm/user/alice"
        );
        assert_eq!(
            create_request["request"]["command"]["actor_ura"],
            "easynet:///r/realm/user/admin"
        );
        assert_eq!(
            create_request["request"]["command"]["idempotency_key"],
            "create-idem"
        );
        assert_eq!(
            create_request["request"]["command"]["proof"]["kind"],
            "bootstrap"
        );
        assert_eq!(
            create_request["request"]["command"]["proof"]["reference"],
            "bootstrap-proof-1"
        );
        assert!(create_request["request"]["command"]
            .get("expected_version")
            .is_none());

        assert_eq!(
            bind_request["request"]["principal_ura"],
            "easynet:///r/realm/user/alice"
        );
        assert_eq!(bind_request["request"]["public_key_b64"], "PUB");
        assert_eq!(bind_request["request"]["key_id"], "key-1");
        assert_eq!(bind_request["request"]["expires_unix_ms"], 12345);
        assert_eq!(
            bind_request["request"]["command"]["idempotency_key"],
            "bind-idem"
        );
        assert_eq!(
            bind_request["request"]["command"]["proof"]["reference"],
            "bootstrap-proof-1"
        );
        assert_eq!(bind_request["request"]["command"]["expected_version"], 1);
    }

    #[test]
    fn bootstrap_request_has_no_product_account_or_private_key_fields() {
        let (create_request, bind_request) = principal_bootstrap_requests(
            FirstKeyRequestInput {
                principal_ura: "easynet:///r/realm/user/alice",
                actor_ura: None,
                create_idempotency_key: "create-idem",
                bind_idempotency_key: "bind-idem",
                key_id: "key-1",
                public_key_b64: "PUB",
                expires_unix_ms: None,
            },
            "bootstrap-proof-1",
        );

        assert!(create_request["request"].get("username").is_none());
        assert!(create_request["request"].get("user_id").is_none());
        assert!(create_request["request"].get("account_id").is_none());
        assert!(create_request["request"].get("session").is_none());
        assert!(bind_request["request"].get("username").is_none());
        assert!(bind_request["request"].get("user_id").is_none());
        assert!(bind_request["request"].get("account_id").is_none());
        assert!(bind_request["request"].get("session").is_none());
        assert!(bind_request["request"].get("private_key").is_none());
        assert!(bind_request["request"].get("seed").is_none());
    }

    #[test]
    fn bootstrap_default_proof_is_recoverable_from_create_idempotency_key() {
        assert_eq!(
            bootstrap_proof_ref(None, "principal-bootstrap-create-1"),
            "proof:principal-bootstrap-create-1"
        );
        assert_eq!(
            bootstrap_proof_ref(
                Some(" explicit-proof ".into()),
                "principal-bootstrap-create-1"
            ),
            "explicit-proof"
        );
    }

    #[test]
    fn enrollment_requests_consume_capability_for_create_and_bind() {
        let (create_request, bind_request) = principal_enrollment_requests(
            FirstKeyRequestInput {
                principal_ura: " easynet:///r/realm/user/bob ",
                actor_ura: None,
                create_idempotency_key: "create-bob",
                bind_idempotency_key: "bind-bob",
                key_id: "key-bob",
                public_key_b64: "BOB_PUB",
                expires_unix_ms: Some(98765),
            },
            " enrollment_1 ",
        );

        assert_eq!(
            create_request["request"]["principal_ura"],
            "easynet:///r/realm/user/bob"
        );
        assert_eq!(
            create_request["request"]["command"]["actor_ura"],
            "easynet:///r/realm/user/bob"
        );
        assert_eq!(
            create_request["request"]["command"]["proof"]["kind"],
            "enrollment"
        );
        assert_eq!(
            create_request["request"]["command"]["proof"]["reference"],
            "enrollment_1"
        );
        assert!(create_request["request"]["command"]
            .get("expected_version")
            .is_none());

        assert_eq!(
            bind_request["request"]["command"]["proof"]["kind"],
            "enrollment"
        );
        assert_eq!(
            bind_request["request"]["command"]["proof"]["reference"],
            "enrollment_1"
        );
        assert_eq!(bind_request["request"]["command"]["expected_version"], 1);
        assert_eq!(bind_request["request"]["key_id"], "key-bob");
        assert_eq!(bind_request["request"]["public_key_b64"], "BOB_PUB");
        assert_eq!(bind_request["request"]["expires_unix_ms"], 98765);
    }

    #[test]
    fn enrollment_requests_have_no_product_account_or_private_key_fields() {
        let (create_request, bind_request) = principal_enrollment_requests(
            FirstKeyRequestInput {
                principal_ura: "easynet:///r/realm/user/bob",
                actor_ura: Some("easynet:///r/realm/user/bob"),
                create_idempotency_key: "create-bob",
                bind_idempotency_key: "bind-bob",
                key_id: "key-bob",
                public_key_b64: "BOB_PUB",
                expires_unix_ms: None,
            },
            "enrollment_1",
        );

        assert!(create_request["request"].get("username").is_none());
        assert!(create_request["request"].get("user_id").is_none());
        assert!(create_request["request"].get("account_id").is_none());
        assert!(create_request["request"].get("session").is_none());
        assert!(bind_request["request"].get("username").is_none());
        assert!(bind_request["request"].get("user_id").is_none());
        assert!(bind_request["request"].get("account_id").is_none());
        assert!(bind_request["request"].get("session").is_none());
        assert!(bind_request["request"].get("private_key").is_none());
        assert!(bind_request["request"].get("seed").is_none());
    }

    #[test]
    fn issue_enrollment_request_uses_issuer_and_subject_without_product_fields() {
        let request = principal_issue_enrollment_request(
            "easynet:///r/realm/user/admin",
            "easynet:///r/realm/user/bob",
            principal_command(
                PrincipalCommandActor::Supplied("easynet:///r/realm/user/admin"),
                "idem-issue",
                None,
                ProofKindArg::ActiveKey,
                "pk_1",
            ),
            Some(1234),
        );

        assert_eq!(
            request["request"]["principal_ura"],
            "easynet:///r/realm/user/admin"
        );
        assert_eq!(
            request["request"]["subject_principal_ura"],
            "easynet:///r/realm/user/bob"
        );
        assert!(request["request"].get("email").is_none());
        assert!(request["request"].get("session").is_none());
        assert_eq!(request["request"]["expires_unix_ms"], 1234);
    }

    #[test]
    fn rotate_key_request_lowers_replacement_as_public_projection() {
        let request = principal_rotate_key_request(
            "easynet:///r/realm/user/alice",
            "binding-1",
            "PUB",
            "key-2",
            principal_command(
                PrincipalCommandActor::Supplied("easynet:///r/realm/user/alice"),
                "idem-rotate",
                Some(3),
                ProofKindArg::ActiveKey,
                "binding-1",
            ),
            Some(9999),
        );

        assert_eq!(
            request["request"]["principal_ura"],
            "easynet:///r/realm/user/alice"
        );
        assert_eq!(request["request"]["binding_id"], "binding-1");
        assert_eq!(request["request"]["replacement"]["public_key_b64"], "PUB");
        assert_eq!(request["request"]["replacement"]["key_id"], "key-2");
        assert_eq!(request["request"]["replacement"]["expires_unix_ms"], 9999);
        assert!(request["request"].get("account_id").is_none());
        assert!(request["request"]["replacement"]
            .get("private_key")
            .is_none());
    }

    #[test]
    fn recover_request_lowers_replacement_key_without_private_material() {
        let request = principal_recover_request(
            "easynet:///r/realm/user/alice",
            "PUB2",
            "key-3",
            principal_command(
                PrincipalCommandActor::subject_self("easynet:///r/realm/user/alice"),
                "idem-recover",
                Some(4),
                ProofKindArg::Recovery,
                "recovery-policy:test",
            ),
            None,
        );

        assert_eq!(
            request["request"]["replacement_key"]["principal_ura"],
            "easynet:///r/realm/user/alice"
        );
        assert_eq!(
            request["request"]["replacement_key"]["public_key_b64"],
            "PUB2"
        );
        assert_eq!(request["request"]["replacement_key"]["key_id"], "key-3");
        assert!(request["request"]["replacement_key"].get("seed").is_none());
    }

    #[test]
    fn issue_grant_request_keeps_generic_action_list() {
        let request = principal_issue_grant_request(
            "easynet:///r/realm/user/admin",
            &[
                "principal.lifecycle.add_key".into(),
                "  principal.lifecycle.recover  ".into(),
            ],
            principal_command(
                PrincipalCommandActor::subject_self("easynet:///r/realm/user/admin"),
                "idem-grant",
                None,
                ProofKindArg::ActiveKey,
                "binding-1",
            ),
            None,
        );

        assert_eq!(
            request["request"]["actions"],
            json!(["principal.lifecycle.add_key", "principal.lifecycle.recover"])
        );
        assert!(request["request"].get("role").is_none());
        assert!(request["request"].get("permission").is_none());
    }

    #[test]
    fn state_request_contains_only_command_and_principal() {
        let request = principal_state_request(
            " easynet:///r/realm/user/alice ",
            principal_command(
                PrincipalCommandActor::subject_self("easynet:///r/realm/user/alice"),
                "idem-state",
                Some(9),
                ProofKindArg::Grant,
                "grant-1",
            ),
        );

        assert_eq!(
            request["request"]["principal_ura"],
            "easynet:///r/realm/user/alice"
        );
        assert!(request["request"].get("next_state").is_none());
    }

    #[test]
    fn bind_first_key_reuses_existing_daemon_managed_key() {
        let existing = managed_key("key-1", "easynet:///r/realm/user/alice", 7);
        let store = FakePrincipalSigningKeyStore::new(
            vec![existing.clone()],
            managed_key("new", "", 8),
            managed_key("rotated", "", 9),
        );

        let key = ensure_principal_signing_key(&store, "easynet:///r/realm/user/alice").unwrap();

        assert_eq!(key.key_id, "key-1");
        assert_eq!(key.public_key_b64, existing.public_key_b64);
        assert_eq!(store.create_calls.get(), 0);
    }

    #[test]
    fn bind_first_key_creates_key_inside_daemon_custody_when_missing() {
        let created = managed_key("key-2", "easynet:///r/realm/user/alice", 9);
        let store = FakePrincipalSigningKeyStore::new(
            Vec::new(),
            created.clone(),
            managed_key("rotated", "", 10),
        );

        let key = ensure_principal_signing_key(&store, "easynet:///r/realm/user/alice").unwrap();

        assert_eq!(key.key_id, "key-2");
        assert_eq!(key.public_key_b64, created.public_key_b64);
        assert_eq!(store.create_calls.get(), 1);
        assert_eq!(
            store.create_subjects.borrow().as_slice(),
            ["easynet:///r/realm/user/alice"]
        );
    }

    #[test]
    fn resolve_principal_signing_key_creates_new_key_for_add_key() {
        let created = managed_key("key-4", "easynet:///r/realm/user/alice", 11);
        let store = FakePrincipalSigningKeyStore::new(
            Vec::new(),
            created.clone(),
            managed_key("rotated", "", 12),
        );

        let key = resolve_principal_signing_key(
            &store,
            "easynet:///r/realm/user/alice",
            KeySource {
                public_key_b64: None,
                key_id: None,
                rotate_from_key_id: None,
            },
        )
        .unwrap();

        assert_eq!(key.key_id, "key-4");
        assert_eq!(store.create_calls.get(), 1);
        assert_eq!(store.rotate_calls.get(), 0);
    }

    #[test]
    fn resolve_principal_signing_key_rotates_inside_daemon_custody() {
        let rotated = managed_key("key-5", "easynet:///r/realm/user/alice", 13);
        let store = FakePrincipalSigningKeyStore::new(
            Vec::new(),
            managed_key("created", "", 14),
            rotated.clone(),
        );

        let key = resolve_principal_signing_key(
            &store,
            "easynet:///r/realm/user/alice",
            KeySource {
                public_key_b64: None,
                key_id: None,
                rotate_from_key_id: Some("old-key"),
            },
        )
        .unwrap();

        assert_eq!(key.key_id, "key-5");
        assert_eq!(key.public_key_b64, rotated.public_key_b64);
        assert_eq!(store.create_calls.get(), 0);
        assert_eq!(store.rotate_calls.get(), 1);
        assert_eq!(store.rotate_subjects.borrow().as_slice(), ["old-key"]);
    }

    #[test]
    fn resolve_principal_signing_key_accepts_public_projection_without_store_call() {
        let store = FakePrincipalSigningKeyStore::new(
            Vec::new(),
            managed_key("created", "", 15),
            managed_key("rotated", "", 16),
        );

        let key = resolve_principal_signing_key(
            &store,
            "easynet:///r/realm/user/alice",
            KeySource {
                public_key_b64: Some("EXTERNAL"),
                key_id: Some("external-key"),
                rotate_from_key_id: None,
            },
        )
        .unwrap();

        assert_eq!(key.key_id, "external-key");
        assert_eq!(key.public_key_b64, "EXTERNAL");
        assert_eq!(store.create_calls.get(), 0);
        assert_eq!(store.rotate_calls.get(), 0);
    }

    struct FakePrincipalSigningKeyStore {
        entries: Vec<ManagedSigningKeyProjection>,
        created: ManagedSigningKeyProjection,
        rotated: ManagedSigningKeyProjection,
        create_calls: Cell<usize>,
        rotate_calls: Cell<usize>,
        create_subjects: RefCell<Vec<String>>,
        rotate_subjects: RefCell<Vec<String>>,
    }

    impl FakePrincipalSigningKeyStore {
        fn new(
            entries: Vec<ManagedSigningKeyProjection>,
            created: ManagedSigningKeyProjection,
            rotated: ManagedSigningKeyProjection,
        ) -> Self {
            Self {
                entries,
                created,
                rotated,
                create_calls: Cell::new(0),
                rotate_calls: Cell::new(0),
                create_subjects: RefCell::new(Vec::new()),
                rotate_subjects: RefCell::new(Vec::new()),
            }
        }
    }

    impl PrincipalSigningKeyStore for FakePrincipalSigningKeyStore {
        fn list(&self) -> anyhow::Result<Vec<ManagedSigningKeyProjection>> {
            Ok(self.entries.clone())
        }

        fn create(&self, principal_ura: &str) -> anyhow::Result<ManagedSigningKeyProjection> {
            self.create_calls.set(self.create_calls.get() + 1);
            self.create_subjects
                .borrow_mut()
                .push(principal_ura.to_string());
            Ok(self.created.clone())
        }

        fn rotate(&self, key_id: &str) -> anyhow::Result<ManagedSigningKeyProjection> {
            self.rotate_calls.set(self.rotate_calls.get() + 1);
            self.rotate_subjects.borrow_mut().push(key_id.to_string());
            Ok(self.rotated.clone())
        }
    }

    fn managed_key(key_id: &str, subject: &str, byte: u8) -> ManagedSigningKeyProjection {
        ManagedSigningKeyProjection {
            key_id: key_id.into(),
            purpose: PROFILE_PURPOSE.into(),
            public_key_b64: B64.encode([byte; 32]),
            status: ManagedSigningStatus::Active,
            rotation_epoch: 0,
            bound_subject: if subject.is_empty() {
                None
            } else {
                Some(subject.into())
            },
            signer_policy_ref: None,
            rotated_from: None,
            created_unix_ms: 1,
            expires_unix_ms: None,
            revoked_unix_ms: None,
        }
    }
}
