//! Before the agent: the gateway.
//!
//! The client asks whether its version is current (0x6100), for notices
//! (0x6104), for the shard list (0x6101, again and again while the title
//! screen is open) and for the farms to ping (0x6106), and then logs in
//! (0x6102). Every request has exactly one answer, which
//! [`GatewayConnection::receive`] returns. A good login gets a token for one
//! agent login ([`World::admit`]) and the agent's address.
//!
//! The gateway touches the world only to admit an account and to count who is
//! online. Checking a password runs argon2 for tens of milliseconds, so in the
//! server it runs on the gateway connection's own task, never in a world step.

use std::net::Ipv4Addr;

use bytes::Bytes;
use packets::gateway::{
    Farm, FarmDetails, NoticeResponse, PatchResponse, Shard, ShardListPingResponse,
    ShardListResponse,
};
use packets::global::ModuleIdentification;
use packets::judge::decode_exact;
use packets::login::{LoginError, LoginInfo, LoginRequest, LoginResponse, WrongAttempt};
use packets::Packet;

use crate::outbox::Frame;
use crate::store;
use crate::wirelog::{Dir, WireLog};
use crate::world::{reason_of, Reason, World};

/// Failed logins a connection may make. Six is the limit the captured vSRO
/// gateway names in its refusals ("attempt n of 6") [V capture, peer vSRO].
/// What the original does after the sixth is unknown; this gateway answers
/// it and closes the connection.
const LOGIN_ATTEMPTS: u32 = 6;

/// The client words code 3 as "This user is already connected" [V original
/// handler].
const ALREADY_CONNECTED: u8 = 3;

/// What the gateway tells clients about this server.
#[derive(Clone, Debug)]
pub struct Gateway {
    pub farm_id: u8,
    pub farm_name: String,
    /// The address the client pings for the farm.
    pub farm_ip: Ipv4Addr,
    pub shard_id: u16,
    pub shard_name: String,
    pub capacity: u16,
    /// Where the agent listens.
    pub agent_ip: String,
    pub agent_port: u16,
}

/// One client's connection to the gateway.
pub struct GatewayConnection {
    log: WireLog,
    failed_logins: u32,
    closed: bool,
}

impl GatewayConnection {
    pub fn open() -> GatewayConnection {
        GatewayConnection {
            log: WireLog::open("gateway"),
            failed_logins: 0,
            closed: false,
        }
    }

    /// Whether the gateway is done with this client: the transport sends the
    /// last answer, then disconnects.
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// One client frame and its answer. A frame that is not exactly a gateway
    /// request closes the connection: before the login there is nothing to
    /// keep.
    pub fn receive(
        &mut self,
        gateway: &Gateway,
        world: &mut World,
        opcode: u16,
        body: Bytes,
    ) -> Result<Frame, Reason> {
        if self.closed {
            return Err(Reason::NotHandled);
        }
        self.log.record(Dir::C2s, opcode, &body);
        let answer = match decode_exact(opcode, &body) {
            Ok(Packet::ModuleIdentification(_)) => {
                Packet::ModuleIdentification(ModuleIdentification {
                    module: "GatewayServer".to_owned(),
                    tail: Some(0),
                    extra: None,
                })
            }
            // There is no download server, so every client is current.
            Ok(Packet::PatchRequest(_)) => Packet::PatchResponse(PatchResponse {
                result: 1,
                error: None,
            }),
            Ok(Packet::NoticeRequest(_)) => {
                Packet::NoticeResponse(NoticeResponse { notice_count: 0 })
            }
            Ok(Packet::ShardListRequest(_)) => shard_list(gateway, world),
            Ok(Packet::ShardListPingRequest(_)) => {
                let farms = vec![Farm {
                    id: gateway.farm_id,
                    ip: gateway.farm_ip,
                }];
                Packet::ShardListPingResponse(ShardListPingResponse { farms })
            }
            Ok(Packet::LoginRequest(request)) => {
                Packet::LoginResponse(self.login(gateway, world, request))
            }
            Ok(_) => return Err(self.close(Reason::NotHandled)),
            Err(verdict) => return Err(self.close(reason_of(verdict))),
        };
        let (opcode, body) = answer.into_serialize();
        self.log.record(Dir::S2c, opcode, &body);
        Ok(Frame { opcode, body })
    }

    fn close(&mut self, reason: Reason) -> Reason {
        self.closed = true;
        reason
    }

    /// An unknown account is refused like a wrong password, deliberately: the
    /// original has a text for "no account info", which would tell a guesser
    /// which names exist. One shard only, so the shard the client names is
    /// the one it was listed.
    fn login(
        &mut self,
        gateway: &Gateway,
        world: &mut World,
        request: LoginRequest,
    ) -> LoginResponse {
        let refused = |error_code, wrong_attempt| LoginResponse {
            result: 2,
            login_info: None,
            login_error: Some(LoginError {
                error_code,
                wrong_attempt,
                account_blocked_err: None,
            }),
            custom: None,
        };
        let Some(account) = store::login(world.store(), &request.username, &request.password)
        else {
            self.failed_logins += 1;
            self.closed = self.failed_logins >= LOGIN_ATTEMPTS;
            let attempt = WrongAttempt {
                max_attempts: LOGIN_ATTEMPTS,
                cur_attempts: self.failed_logins,
            };
            return refused(1, Some(attempt));
        };
        if world.connected(account) {
            return refused(ALREADY_CONNECTED, None);
        }
        let agent = LoginInfo {
            agent_token: world.admit(account, &request.username),
            agent_ip: gateway.agent_ip.clone(),
            agent_port: gateway.agent_port,
        };
        // The captured vSRO gateway appends five zero bytes the client never
        // reads [V capture, peer vSRO; V original handler]; they stay off.
        LoginResponse {
            result: 1,
            login_info: Some(agent),
            login_error: None,
            custom: None,
        }
    }
}

fn shard_list(gateway: &Gateway, world: &World) -> Packet {
    let farms = vec![FarmDetails {
        id: gateway.farm_id,
        name: gateway.farm_name.clone(),
    }];
    let shards = vec![Shard {
        id: gateway.shard_id,
        name: gateway.shard_name.clone(),
        online_count: u16::try_from(world.online()).unwrap_or(u16::MAX),
        capacity: gateway.capacity,
        is_operating: true,
        farm_id: gateway.farm_id,
    }];
    Packet::ShardListResponse(ShardListResponse { farms, shards })
}
