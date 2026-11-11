//! This machine's local door, decided through xenonet-authz: what the HTTP
//! routes' rules table (xenonet-http's routes/rules.rs) asks before a
//! route's handler runs, as `Cores::door`, which the composition root puts
//! in place. Local acts are decided with no
//! network: nothing here is another machine's, so only who asks and how
//! they were checked matter, and the machine's name doesn't.

use std::sync::Arc;
use xenonet_api::cores::door::{self, Caller, LocalDoor};
use xenonet_authz::{
    Act, AgentId, AllowScope, AskId, Asked, Attestation, Consent, Device, DeviceId, Door, Ground, MachineId, NetworkId, PersonId, Principal, ReadScope, Request, Resource, Role,
    RootCertificate, SessionId, Settings, StandingGrant,
};

/// This machine as its own local door sees it.
struct Here(MachineId);

impl Ground for Here {
    fn own_machine(&self) -> &MachineId {
        &self.0
    }
    fn role(&self, _: &NetworkId, _: &MachineId) -> Option<Role> {
        None
    }
    fn settings(&self, _: Option<&NetworkId>) -> Settings {
        Settings::default()
    }
    fn own_person(&self) -> Option<PersonId> {
        None
    }
    fn device(&self, _: &PersonId, _: &DeviceId) -> Option<Device> {
        None
    }
    fn revoked(&self, _: &[u8]) -> bool {
        false
    }
    fn token_roots(&self, _: u32) -> Vec<RootCertificate> {
        Vec::new()
    }
    fn signing_key(&self, _: &MachineId) -> Option<xenonet_id::sign::VerifyingKey> {
        None
    }
    fn consent(&self, _: &SessionId) -> Consent {
        Consent::default()
    }
    fn device_of(&self, _: &MachineId) -> Option<DeviceId> {
        None
    }
    fn delegations(&self) -> Vec<StandingGrant> {
        Vec::new()
    }
    fn allow_scope(&self, _: &[u8]) -> Option<AllowScope> {
        None
    }
    fn ask(&self, _: &AskId) -> Option<Asked> {
        None
    }
    fn governance_synced_at(&self) -> Option<i64> {
        None
    }
}

fn here() -> Option<MachineId> {
    MachineId::new("this-machine").ok()
}

fn scope(s: door::ReadScope) -> ReadScope {
    match s {
        door::ReadScope::Map => ReadScope::Map,
        door::ReadScope::Transcripts => ReadScope::Transcripts,
        door::ReadScope::Search => ReadScope::Search,
        door::ReadScope::State => ReadScope::State,
    }
}

fn act(a: door::Act) -> Act {
    match a {
        door::Act::Read(s) => Act::Read { scope: scope(s), filters: None },
        door::Act::Message => Act::Message,
        door::Act::Claim => Act::Claim,
        door::Act::Handoff => Act::Handoff,
    }
}

/// The local door, through xenonet-authz.
#[derive(Clone, Copy, Debug, Default)]
pub struct Local;

impl LocalDoor for Local {
    fn decide(&self, caller: &Caller, a: door::Act, now: f64) -> Result<(), String> {
        let machine = here().ok_or("this machine has no id to decide by")?;
        let principal = match caller {
            Caller::Machine => Principal::Machine { id: machine.clone() },
            Caller::Agent(id, attestation) => {
                let id = AgentId::new(id).or_else(|_| AgentId::new("unchecked")).map_err(|e| e.message)?;
                let attestation = match attestation {
                    door::Attestation::Unchecked => Attestation::Unchecked,
                    door::Attestation::ProcessChecked => Attestation::ProcessChecked,
                };
                Principal::Agent { id, machine: machine.clone(), attestation }
            }
        };
        let request = Request::new(principal, Resource::Machine(machine.clone()), act(a), Door::Local, now as i64, None);
        match xenonet_authz::decide(None, &request, &Here(machine), None) {
            xenonet_authz::Decision::Allow(_) => Ok(()),
            d => Err(d.reason().map_or_else(|| "not allowed".to_string(), |r| r.text().to_string())),
        }
    }
}

/// The local door, for `Cores::door`.
pub fn local() -> Arc<dyn LocalDoor> {
    Arc::new(Local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_api::cores::door::{Act as DoorAct, Attestation as Checked, ReadScope as Scope};

    #[test]
    fn reads_that_matter_need_more_than_the_file() {
        let d = Local;
        for scope in [Scope::Transcripts, Scope::Search, Scope::State] {
            assert!(d.decide(&Caller::Machine, DoorAct::Read(scope), 0.0).is_ok());
            assert!(d.decide(&Caller::Agent("claude:s".into(), Checked::ProcessChecked), DoorAct::Read(scope), 0.0).is_ok());
            let why = d.decide(&Caller::Agent(String::new(), Checked::Unchecked), DoorAct::Read(scope), 0.0).unwrap_err();
            assert!(why.contains("unchecked agent"), "{why}");
        }
        // The map, a message, a claim and a handoff are anyone's here.
        let anyone = Caller::Agent(String::new(), Checked::Unchecked);
        for act in [DoorAct::Read(Scope::Map), DoorAct::Message, DoorAct::Claim, DoorAct::Handoff] {
            assert!(d.decide(&anyone, act, 0.0).is_ok());
        }
    }
}
