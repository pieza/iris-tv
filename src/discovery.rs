use crate::config::AppConfig;
use crate::errors::IrisError;
use crate::profiles::Profile;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::IpAddr;

pub const IRIS_SERVICE_TYPE: &str = "_iris-tv._tcp.local.";
const API_VERSION: &str = "1";

pub struct DiscoveryAdvertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for DiscoveryAdvertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

pub fn should_advertise(config: &AppConfig) -> bool {
    if !config.discovery_enabled {
        return false;
    }

    config
        .server_host
        .parse::<IpAddr>()
        .map(|ip| !ip.is_loopback())
        .unwrap_or(false)
}

pub fn register(
    profile: &Profile,
    config: &AppConfig,
) -> Result<Option<DiscoveryAdvertisement>, IrisError> {
    if !should_advertise(config) {
        return Ok(None);
    }

    let service = build_service_info(profile, config)?;
    let fullname = service.get_fullname().to_string();
    let daemon = ServiceDaemon::new().map_err(|err| IrisError::Discovery(err.to_string()))?;
    daemon
        .register(service)
        .map_err(|err| IrisError::Discovery(err.to_string()))?;
    Ok(Some(DiscoveryAdvertisement { daemon, fullname }))
}

pub fn build_service_info(profile: &Profile, config: &AppConfig) -> Result<ServiceInfo, IrisError> {
    let device_id = config
        .device_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or("iris-tv");
    let instance_name = if config.device_name.trim().is_empty() {
        format!("IRIS {} TV", profile.brand)
    } else {
        config.device_name.clone()
    };
    let host_name = format!("{device_id}.local.");
    let auth_required = config
        .api_token
        .as_deref()
        .is_some_and(|token| !token.is_empty())
        .to_string();
    let properties = [
        ("id", device_id.to_string()),
        ("name", instance_name.clone()),
        ("brand", profile.brand.clone()),
        ("model", profile.model.clone()),
        ("api_version", API_VERSION.to_string()),
        ("auth_required", auth_required),
    ];

    ServiceInfo::new(
        IRIS_SERVICE_TYPE,
        &instance_name,
        &host_name,
        "0.0.0.0",
        config.server_port,
        &properties[..],
    )
    .map(|service| service.enable_addr_auto())
    .map_err(|err| IrisError::Discovery(err.to_string()))
}
