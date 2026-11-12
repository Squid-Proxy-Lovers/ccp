// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{
    State,
    dispatch::{decode, id, role},
    request::Input,
    response::{self, Error, Reply},
};
use crate::foundation::{AccessStore, Actor, LinkAdmission, LinkInvitationRequest, LinkLifetime};
use hyper::Method;
use protocol::{
    enrollment_link::{EnrollmentLink, LinkTarget},
    workspace::invitation_links as api,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;
pub(super) fn handles(path: &str) -> bool {
    path == "/api/v1/admin/invitations/link"
}
pub(super) fn dispatch(
    state: &State,
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    if input.method != Method::POST {
        return Err(Error::Method);
    }
    let body: api::Issue = decode(&input.body)?;
    let workspace = id(&body.workspace_id)?;
    let lifetime = match body.lifetime {
        api::Lifetime::Short {} => LinkLifetime::Short,
        api::Lifetime::At {
            expires_at,
            confirm_long_lived,
        } => LinkLifetime::At {
            expires_at,
            confirmed: confirm_long_lived,
        },
        api::Lifetime::Never { confirm_no_expiry } => LinkLifetime::Never {
            confirmed: confirm_no_expiry,
        },
    };
    let expires_at = lifetime
        .expires_at(now)
        .map_err(|_expiry| Error::BadRequest)?;
    let info = store
        .workspace_administration(actor, workspace, now)
        .map_err(operation)?;
    let target = target(
        state,
        input,
        &info.workspace.name,
        workspace,
        body.https_origin,
        body.mtls_origin,
    )?
    .validated()
    .map_err(|_metadata| Error::BadRequest)?;
    let (admission, max_uses) = match body.admission {
        api::Admission::Individual {} => (LinkAdmission::Individual, Some(1)),
        api::Admission::InitialGroup { max_clients } => (
            LinkAdmission::InitialGroup { max_clients },
            max_clients.map(u64::from),
        ),
    };
    if max_uses == Some(0) || (max_uses.is_none() && lifetime.long_lived(now)) {
        return Err(Error::BadRequest);
    }
    // Domain issuance rechecks current permission and lifecycle inside its atomic
    // transaction. Validate all URI metadata first, before minting a bearer.
    let issued = store
        .issue_link_invitation(
            actor,
            LinkInvitationRequest {
                workspace_id: workspace,
                role: role(body.role),
                admission,
                lifetime,
            },
            now,
        )
        .map_err(operation)?;
    let workspace_name = target.workspace_name.clone();
    let link =
        EnrollmentLink::new(target, issued.secret()).map_err(|_invariant| Error::Unavailable)?;
    response::success(api::Issued {
        invitation_id: issued.id.to_string(),
        enrollment_link: link.to_uri(),
        workspace_id: workspace.to_string(),
        workspace_name,
        expires_at,
        max_uses,
    })
}
fn target(
    state: &State,
    input: &Input,
    name: &str,
    workspace: Uuid,
    https_override: Option<String>,
    override_origin: Option<String>,
) -> Result<LinkTarget, Error> {
    let https_origin = https_override.unwrap_or_else(|| input.target_origin.clone());
    let mut mtls = url::Url::parse(&https_origin).map_err(|_origin| Error::BadRequest)?;
    mtls.set_port(Some(state.mtls_port))
        .map_err(|()| Error::Unavailable)?;
    let trust = state.identity.trust();
    let der = crate::foundation::strict_pem_der(&trust.ca_certificate_pem, "CERTIFICATE")
        .map_err(|_ca| Error::Unavailable)?;
    Ok(LinkTarget {
        https_origin,
        mtls_origin: override_origin.unwrap_or_else(|| mtls.to_string()),
        instance_id: Uuid::parse_str(&trust.instance_id).map_err(|_id| Error::Unavailable)?,
        ca_sha256: Sha256::digest(der).into(),
        workspace_id: workspace,
        workspace_name: name.to_owned(),
    })
}
fn operation(error: anyhow::Error) -> Error {
    response::operation(error, Error::Forbidden)
}
