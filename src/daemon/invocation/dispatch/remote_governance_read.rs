//! Remote governance-read route admission.
//!
//! Receipt history and runtime catalogue abilities are daemon governance reads,
//! not product actions. Every remote carrier must apply the same selected-route
//! tuple policy before forwarding to presence so direct SDK/FFI ingress cannot
//! bypass the typed issuers used by CLI facades.

use axon_sdk::pb::axon::v1::Envelope;
use tonic::Status;

use crate::daemon::invocation::routing::route_resolver::SelectedInvokeRoute;

pub(crate) fn require_remote_governance_read_route(
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
    if subject_ura == route.callee_ura {
        return Err(Status::failed_precondition(format!(
            "CANONICAL_HISTORY_READ_REQUIRED: {surface} receipt-history ability \
             `{history_ability}` must not use target-owned subject `{}`; use the canonical \
             receipt-history read path",
            route.callee_ura
        )));
    }
    let subject = crate::core::identity::RuntimeIdentityUra::parse(subject_ura).map_err(|err| {
        Status::invalid_argument(format!("{surface} receipt-history read subject_ura {err}"))
    })?;
    if subject.kind() == crate::core::ura::URAKind::Resource {
        return Ok(());
    }

    Err(Status::failed_precondition(format!(
        "CANONICAL_HISTORY_READ_REQUIRED: {surface} receipt history ability `{history_ability}` \
         must use a resource read-model subject; use the canonical invocation history read path"
    )))
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
    if subject_ura == route.callee_ura {
        return Ok(());
    }

    Err(Status::failed_precondition(format!(
        "CANONICAL_CATALOGUE_READ_REQUIRED: {surface} remote catalogue ability \
         `{catalogue_ability}` must use runtime-read subject `{}`; use the canonical remote \
         catalogue read path",
        route.callee_ura
    )))
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
