use std::{fs::File, io::BufReader, collections::HashMap, sync::{Mutex, Arc}, time::Duration};
use clokwerk::Interval;
use flexi_logger::{LoggerHandle, Logger, Criterion, FileSpec, Naming, Cleanup, Duplicate};
use gateway::{SensorGateway, SensorData, SensorValue};
use rumqttc::{MqttOptions, Client, QoS, NetworkOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;

mod gateway;

#[derive(Debug, Deserialize, Clone)]
struct SensorConfig {
    class: Option<String>,
    unit: Option<String>,
    value_template: Option<String>,
    name: Option<String>,
    json_attributes_topic: Option<String>,
    json_attributes_template: Option<String>,
}

impl SensorConfig {
    pub fn new() -> Self {
        SensorConfig {
            class: Option::None,
            unit: Option::None,
            value_template: Option::None,
            name: Option::None,
            json_attributes_topic: Option::None,
            json_attributes_template: Option::None,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct DiscoverySensor {
    name: String,
    state_topic: String,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    json_attributes_topic: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    json_attributes_template: Option<String>,

    #[serde(rename = "uniq_id")]
    unique_id: String,

    #[serde(rename = "dev_cla")]
    #[serde(skip_serializing_if = "Option::is_none")]
    device_class: Option<String>,

    #[serde(rename = "unit_of_meas")]
    #[serde(skip_serializing_if = "Option::is_none")]
    unit_of_measurement: Option<String>,

    #[serde(rename = "val_tpl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    value_template: Option<String>,
}

impl DiscoverySensor {
    pub fn new(id: String, name: String, topic: String, sensor_config: &SensorConfig) -> Self {
        DiscoverySensor {
            name: name.clone(),
            state_topic: topic,
            unique_id: format!("{}_{}", id.clone(), name.clone()),
            device_class: sensor_config.class.clone(),
            unit_of_measurement: sensor_config.unit.clone(),
            value_template: sensor_config.value_template.clone(),
            json_attributes_template: sensor_config.json_attributes_template.clone(),
            json_attributes_topic: sensor_config.json_attributes_topic.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct DiscoveryOrigin {
    name: String,
    sw: String,
}

impl DiscoveryOrigin {
    pub fn new() -> Self {
        DiscoveryOrigin {
            name: env!("CARGO_PKG_NAME").to_string(),
            sw: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct DiscoverySensorDevice {
    name: String,
    #[serde(rename = "sw")]
    sw_version: String,
    #[serde(rename = "ids")]
    identifiers: Vec<String>,
    #[serde(rename = "mdl")]
    model: String,
}

impl DiscoverySensorDevice {
    fn new(gw: &SensorGateway) -> Self {
        DiscoverySensorDevice {
            identifiers: vec![
                gw.name(),
            ],
            model: gw.version(),
            name: gw.name(),
            sw_version: gw.firmware(),
        }
    }
}

#[derive(Debug, Serialize)]
struct DiscoverySensorPayload {
    #[serde(flatten)]
    sensor: DiscoverySensor,
    #[serde(rename = "dev")]
    device: DiscoverySensorDevice,
    #[serde(rename = "o")]
    origin: DiscoveryOrigin,
}

impl DiscoverySensorPayload {
    fn new(sensor: DiscoverySensor, device: DiscoverySensorDevice) -> Self {
        DiscoverySensorPayload {
            sensor: sensor,
            device: device,
            origin: DiscoveryOrigin::new(),
        }
    }
}

struct Gateway {
    mqtt: Arc<Mutex<Client>>,
    gateway: SensorGateway,
    sensor_config: Mutex<HashMap<String, SensorConfig>>,
    discovered_sensor: Mutex<HashMap<String, DiscoverySensor>>,
}

struct Gateways {
    gateways: HashMap<String, Gateway>,
    _mqtt: Arc<Mutex<Client>>,
}

impl Gateway {
    fn new(ip : String, sensor_config: HashMap<String, SensorConfig>, mqtt: Arc<Mutex<Client>>) -> Self {
        let gateway = SensorGateway::new(ip, 45000);
        Gateway {
            gateway: gateway,
            sensor_config: Mutex::new(sensor_config),
            discovered_sensor: Mutex::new(HashMap::new()),
            mqtt: mqtt,
        }
    }

    pub fn gateway(&self) -> &SensorGateway {
        &self.gateway
    }

    fn get_sensor_name(&self, sensor: &SensorData, config: &SensorConfig) -> String {
        if config.name.is_some() {
            config.name.clone().unwrap()
        } else {
            sensor.name().to_string()
        }
    }

    pub fn sensor_topic(&self, _sensor: &SensorData, _config: &SensorConfig) -> String {
        format!("awgateway/{}/data", self.gateway.name())
    }

    fn sent_discovery(&self, name: &str) -> bool {
        let l_discovered: std::sync::MutexGuard<'_, HashMap<String, DiscoverySensor>> = self.discovered_sensor.lock().expect("Failed to lock discovery mutex");
        l_discovered.contains_key(name)
    }
    
    fn build_discovery_payload_from_sensor_data(&self, sensor: &SensorData, config: &SensorConfig) -> DiscoverySensorPayload {
        let dsensor: DiscoverySensor = DiscoverySensor::new(self.gateway().name(), self.get_sensor_name(sensor, config), self.sensor_topic(sensor, config), config);     
        DiscoverySensorPayload::new(dsensor.clone(), DiscoverySensorDevice::new(self.gateway()))
    }

    fn send_discovery_sensor(&self, name: &str, payload: &DiscoverySensorPayload) -> Result<bool, String> {
        let json_str = serde_json::to_string(&payload).unwrap();
        if let Err(e) = self.mqtt.lock().unwrap().publish(
                format!("homeassistant/sensor/{}/config", 
                payload.sensor.unique_id),
                QoS::AtLeastOnce,
                true,
                json_str.clone()) {
            log::error!("Failed to send discovery message - error {:?}", e);
            return Err(format!("Error={:?}", e));
        }

        log::debug!("Send discovery message: {:?}", json_str);

        let mut discover = self.discovered_sensor.lock().expect("Failed to lock discovery mutex");
        discover.insert(name.to_string(), payload.sensor.clone());

        Ok(true)
    }

    pub fn update_metadata(&self) {
        let mut sent_msgs = 0;
        let mut sent_disc = 0;

        log::info!("Updating metadata for {}", self.gateway.name());

        // TODO: handle when a sensor goes away

        // Send discovery (if needed) and data for battery/signal
        let metadata = self.gateway.update_sensor_metadata().unwrap();
        for meta in metadata {
            if let Some(bat_state) = meta.1.battery_state {
                let field = format!("{}", meta.1.type_id_str);
                let name = format!("{}_info", field);
                let topic = format!("awgateway/{}/{}/info", self.gateway.name(), &field);

                if !self.sent_discovery(&name) {
                    // Format discovery message for battery/signal metadata
                    let value_temp = format!("{{{{ value_json.{} | default(\"\") }}}}", "battery_status");

                    let mut config = SensorConfig::new();
                    config.name = Some(name.clone());
                    config.value_template = Some(value_temp.clone());
                    config.json_attributes_topic = Some(topic.clone());
            
                    let dsensor: DiscoverySensor = DiscoverySensor::new(self.gateway().name(), name.clone(), topic.clone(), &config);     
                    let payload = DiscoverySensorPayload::new(dsensor.clone(), DiscoverySensorDevice::new(self.gateway()));

                    let res = self.send_discovery_sensor(&name, &payload);
                    if res.is_err() {
                        log::error!("Failed to send discovery for {}:{:?}, skipping data", self.gateway().name(), name);
                        continue;
                    } else {
                        sent_disc += 1;
                    }
                }

                // Send data for metadata
                let mut vals: HashMap<String, serde_json::Value> = HashMap::new();
                vals.insert("battery_status".to_string(), SensorValue::to_json_val(&SensorValue::Battery(bat_state)));
                vals.insert("signal".to_string(), json!(meta.1.signal));

                let json_str = serde_json::to_string(&vals).unwrap();
                log::debug!(" Sending json {:?} for sensor metadata", json_str.clone());
        
                if let Err(e) = self.mqtt.lock().unwrap().publish(
                    topic.clone(),
                    QoS::AtLeastOnce,
                    false,
                    json_str.clone()) {
                    log::error!("Failed to send metadata message - error {:?}", e);
                } else {
                    sent_msgs += 1;
                }
            }
        }
        log::info!("Metadata updated {} values and sent {} discovery messages", sent_msgs, sent_disc);
    }

    pub fn update_livedata(&self) {
        let mut sent_msgs = 0;

        self.update_metadata();

        log::info!("Updating live data for {}", self.gateway.name());
        let data = match self.gateway.get_live_data() {
            Ok(data) => data,
            Err(err) => {
                log::error!("Failed to get live data - error {:?}", err);
                return;
            }
        };

        log::debug!(" Checking for discovery for sensors");

        let mut vals: HashMap<String, serde_json::Value> = HashMap::new();
        for sensors in data {
            for sensor in sensors {
                let mut config_lock: std::sync::MutexGuard<'_, HashMap<String, SensorConfig>> = self.sensor_config.lock().expect("Failed to get sensor config lock");

                let config_opt = config_lock.get_mut(sensor.name());
                if config_opt.is_none() {
                    log::debug!("Failed to find sensor config for {}:{} - value {:?}", self.gateway().name(), sensor.name(), sensor.value());
                    // only send data for sensors in the sensor config
                    continue;
                }

                // Check if we need to send HA auto discovery for the sensor
                let config = config_opt.unwrap();

                if !self.sent_discovery(sensor.name()) {
                    let payload = self.build_discovery_payload_from_sensor_data(&sensor, config);
                    let res = self.send_discovery_sensor(&sensor.name(), &payload);
                    if res.is_err() {
                        log::error!("Failed to send discovery for {}:{:?}, skipping data", self.gateway().name(), sensor.name());
                        continue;
                    }
                    sent_msgs += 1;
                }
        
                vals.insert(self.get_sensor_name(&sensor, config), SensorValue::to_json_val(sensor.value()));
            }
        }

        let json_str = serde_json::to_string(&vals).unwrap();
        log::debug!(" Sending json {:?} for sensor data", json_str.clone());

        if let Err(e) = self.mqtt.lock().unwrap().publish(
            format!("awgateway/{}/data", self.gateway.name()),
            QoS::AtLeastOnce,
            false,
            json_str.clone()) {
            log::error!("Failed to send data message - error {:?}", e);
        }

        log::info!("Updated {} values and sent {} discovery messages", vals.len(), sent_msgs);

    }

}

impl Gateways {
    fn new(config: &config::Config) -> Result<Self, String> {
        let mqtt_host = config.get_string("mqtt.host").expect("Failed to find mqtt.host config");
        let mqtt_user = config.get_string("mqtt.user");
        let mqtt_psw = config.get_string("mqtt.password");
        let mqtt_keepalive = config.get_int("mqtt.keep_alive").unwrap_or(20);

        let mut options = MqttOptions::parse_url(mqtt_host.clone()).expect("failed to init MqttOptions");

        options.set_keep_alive(Duration::from_secs(mqtt_keepalive as u64))
                .set_clean_session(true);
                
        if mqtt_user.is_ok() {
            options.set_credentials(mqtt_user.unwrap(), mqtt_psw.expect("mqtt user is set, expect password"));
        }

        let (client, mut connection) = Client::new(options.clone(), 10);

        let mut net_options = NetworkOptions::new();
        net_options.set_connection_timeout(15);
        connection.eventloop.set_network_options(net_options);

        log::info!("Connected to {}", mqtt_host.clone());

        let p_mqtt = Arc::new(Mutex::new(client));

        // Create thread for event loop for mqtt
        std::thread::spawn(move || {
            loop {
                for (_i, notification) in connection.iter().enumerate() {
                    match notification {
                        Ok(event) => log::trace!("Received {:?} from mqtt", event),
                        Err(err) => {
                            log::error!("Ending program, MQTT error {:?}", err);
                            std::process::exit(1);
                        }
                    }
                }
            }
        });

        Ok(Gateways {
            gateways: Self::parse_gateways(config, p_mqtt.clone()),
            _mqtt: p_mqtt,
        })
    }

    pub fn update_livedata(&self) {
        for gateway in &self.gateways {
            gateway.1.update_livedata();
        }
    }

    fn parse_gateways(config: &config::Config, mqtt: Arc<Mutex<Client>>) -> HashMap<String, Gateway> {
        let mut gateways = HashMap::new();

        let gateways_vec: Vec<String>;
        if let Ok(gateway) = config.get_string("config.gateways") {
            // Read gateways as string, split, and convert to array of string
            gateways_vec = gateway.split(",").map(|v| v.to_string()).collect();
        } else {
            // read gateways as array, and convert into vector of string
            gateways_vec = config.
                        get_array("config.gateways").expect("Missing gateways config").
                        iter().map(|v| v.clone().into_string().unwrap()).collect();
        }
    
        // Global json sensor config
        let file = File::open(
            &config
                .get_string("config.sensors")
                .expect("failed to get sensor definitions"),
            ).expect("unable to open def file");
        let sensor_config: HashMap<String, SensorConfig> = serde_json::from_reader(BufReader::new(file)).expect("failed to parse global sensor definitions");
    
        for gateway in gateways_vec {
            let mut gw_sensor_config = sensor_config.clone();
            let sensor_config_file = &config.get_string(&format!("{}.sensors", gateway));

            if sensor_config_file.is_ok() {
                // Read local config for the gateway
                let gw_sensor_file = File::open(
                    &config
                        .get_string(&format!("{}.sensors", gateway))
                        .expect("failed to get sensor definitions"),
                    ).expect("Failed to find sensor configuration");

                let local_sensor_config: HashMap<String, SensorConfig> = serde_json::from_reader(BufReader::new(gw_sensor_file)).expect(&format!("failed to parse {} sensor definitions", gateway));

                // Merge config
                for config in local_sensor_config {
                    if gw_sensor_config.contains_key(&config.0) {
                        gw_sensor_config.remove(&config.0);
                    }
                    gw_sensor_config.insert(config.0, config.1);
                }
            }

            let gw = Gateway::new(gateway.clone(), gw_sensor_config, mqtt.clone());
            gateways.insert(gateway.clone(), gw);
        }

        gateways
    }
}

fn get_log_level(level: String) -> Duplicate {
    let console_level_str = match std::env::var("RUST_LOG") {
        Ok(val) => val,
        _ => level,
    };

    let console_level = match console_level_str.to_lowercase().as_str() {
        "none" => Duplicate::None,
        "warn" => Duplicate::Warn,
        "error" => Duplicate::Error,
        "info" => Duplicate::Info,
        "debug" => Duplicate::Debug,
        "trace" => Duplicate::Trace,
        "all" => Duplicate::All,
        _ => Duplicate::Info,
    };

    console_level
}

fn setup_logging(config: &config::Config) -> Result<LoggerHandle, Box<dyn std::error::Error>> {    
    let files = config.get_int("config.files").unwrap_or(5);
    let rotate_size = byte_unit::Byte::parse_str(
        config.get_string("config.rotate_size").unwrap_or("50MB".to_string()),
        true
    ).expect("Failed to parse rotate size").as_u64();
    
    let logdir = config.get_string("log.directory").unwrap_or("logs".to_string());
    let stdout_level = config.get_string("log.stdout_level").unwrap_or("info".to_string());
    let logfile_level = config.get_string("log.logfile_level").unwrap_or("debug".to_string());

    let ret = Logger::try_with_env_or_str(logfile_level)?
            .duplicate_to_stdout(get_log_level(stdout_level))
            .format_for_stdout(flexi_logger::opt_format)
            .format_for_files(flexi_logger::opt_format)
            .cleanup_in_background_thread(true)
            .log_to_file(
                FileSpec::default()
                    .directory(logdir)
                    .basename("service")

            )
            .append()
            .rotate(
                Criterion::Size(rotate_size), 
                Naming::Timestamps, 
                Cleanup::KeepCompressedFiles(files as usize))
            .start()?;

    Ok(ret)
}

pub fn path_exists(path: &str) -> bool {
    std::fs::metadata(path).is_ok()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings;
    if path_exists("/config") {
        settings = "/config/settings";
    } else {
        settings = "settings";
    }

    // Read configuration
    let settings: config::Config = config::Config::builder()
        .add_source(config::File::with_name(settings))
        .build()
        .expect("failed to read Settings.toml");

    // Keep alive log until end of main
    let _log_handle: LoggerHandle = setup_logging(&settings).expect("Failed to setup logging");

    let gw = Gateways::new(&settings).unwrap();

    let poll_interval_sec = settings.get_int("config.poll_interval_sec").expect("Missing poll_interval_sec in the configuration");

    // Run one update first
    log::info!("Running first update livedata for all gateways");
    gw.update_livedata();

    let mut scheduler = clokwerk::Scheduler::new();
    scheduler.every(Interval::Seconds(poll_interval_sec as u32)).run(move || {
        log::info!("Running update livedata for all gateways");
        gw.update_livedata()
    });

    // Run forever
    loop {
        scheduler.run_pending();
        std::thread::sleep(Duration::from_millis(10000));
    }
}
