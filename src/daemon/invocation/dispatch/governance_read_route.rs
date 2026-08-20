//! Selected-route governance-read admission.
//!
//! Receipt history and runtime catalogue abilities are daemon governance reads,
//! not product actions. Every selected-route dispatcher must apply the same
//! tuple policy before forwarding to presence or entering LocalRuntime so direct
//! SDK/FFI ingress cannot bypass the typed issuers used by CLI facades.

use axon_sdk::pb::axon::v1::Envelope;
use tonic::Status;

use crate::daemon::invocation::routing::route_resolver::SelectedInvokeRoute;

pub(crate) fn require_selected_governance_read_route(
    surface: &'static str,
    route: &SelectedInvokeRoute,
    envelope: &Envelope,
) -> Result<(), Status> {
    require_receipt_history_read_subject(surface, route, envelope)?;
    require_catalogue_read_subject(surface, route, envelope)
}

fn require_receipt_history_read_subject(
    surface: &'static str,
    route: &SelectedInvokeRoute,
    envelope: &Envelope,
) -> Result<(), Status> {
    let history_ability = selected_route_public_ability(route).filter(|ability| {
        crate::daemon::ability::names::governance::is_invocation_history_read(ability)
    });

    let Some(history_ability) = history_ability else {
        return Ok(());
    };
    if surface != "Invoke" {
        return Err(Status::failed_precondition(format!(
            "CANONICAL_HISTORY_READ_REQUIRED: {surface} remote receipt-history ability \
             `{history_ability}` must enter through canonical unary Invoke receipt-history path"
        )));
    }

    let subject_ura = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{surface} receipt-history read envelope is missing read-model subject"
            ))
        })?;
    let read_subject = crate::core::identity::RuntimeGovernanceReadSubject::parse_for_callee(
        subject_ura,
        &route.callee_ura,
    )
    .map_err(|err| {
        Status::failed_precondition(format!(
            "CANONICAL_HISTORY_READ_REQUIRED: {surface} receipt history ability \
             `{history_ability}` must use a runtime governance read subject; \
             use the canonical invocation history read path: {err}"
        ))
    })?;
    if read_subject.as_str() == route.callee_ura {
        let caller_ura = envelope
            .caller
            .as_ref()
            .map(|caller| caller.ura.trim())
            .unwrap_or_default();
        if caller_ura != crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
            return Err(Status::failed_precondition(format!(
                "CANONICAL_HISTORY_READ_REQUIRED: {surface} receipt history ability \
                 `{history_ability}` must enter runtime-owner reads through the canonical \
                 local-system governance issuer; use the canonical invocation history read path"
            )));
        }
    }
    Ok(())
}

fn require_catalogue_read_subject(
    surface: &'static str,
    route: &SelectedInvokeRoute,
    envelope: &Envelope,
) -> Result<(), Status> {
    let Some(catalogue_ability) = selected_route_public_ability(route).filter(|ability| {
        crate::daemon::ability::names::governance::is_runtime_catalogue_read(ability)
    }) else {
        return Ok(());
    };
    if surface != "Invoke" {
        return Err(Status::failed_precondition(format!(
            "CANONICAL_CATALOGUE_READ_REQUIRED: {surface} remote catalogue ability \
             `{catalogue_ability}` must enter through canonical unary Invoke catalogue read path"
        )));
    }

    let subject_ura = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{surface} catalogue read envelope is missing runtime-read subject"
            ))
        })?;
    crate::core::identity::RuntimeGovernanceReadSubject::parse_for_callee(
        subject_ura,
        &route.callee_ura,
    )
    .map(|_| ())
    .map_err(|err| {
        Status::failed_precondition(format!(
            "CANONICAL_CATALOGUE_READ_REQUIRED: {surface} remote catalogue ability \
             `{catalogue_ability}` must use a runtime governance read subject; \
             use the canonical remote catalogue read path: {err}"
        ))
    })
}

fn selected_route_public_ability(route: &SelectedInvokeRoute) -> Option<String> {
    let projected =
        crate::core::ura::owner_local_ability_name(&route.callee_ura, &route.dispatch_name);
    if !projected.trim().is_empty() {
        return Some(projected);
    }
    crate::core::ura::AbilitySelector::parse(&route.ability_ura)
        .ok()
        .map(|selector| selector.public_name().to_string())
}
