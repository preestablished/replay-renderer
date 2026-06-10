#![forbid(unsafe_code)]

pub fn ping() -> replay_proto::replay::agent::v1::PingResponse {
    replay_proto::replay::agent::v1::PingResponse {
        version: "0.1.0".to_string(),
        hypervisor_reachable: false,
    }
}
