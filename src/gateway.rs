//
// Protocol:
//   https://osswww.ecowitt.net/uploads/20210716/WN1900%20GW1000,1100%20WH2680,2650%20telenet%20v1.6.0%20.pdf
//
use std::{collections::HashMap, time::Duration};
use std::net::{TcpStream, SocketAddr, Ipv4Addr};
use std::str::{self, FromStr};
use std::io::{Read, Write, Error, ErrorKind};
use std::thread::sleep;

use serde_json::{json, Value};

const HEADER: &'static [u8] = &[ 0xFF, 0xFF];

#[derive(Debug)]
pub struct SensorGateway {
    firmware: Option<String>,
    mac_address: Option<String>,
    
    max_tries: u32,
    retry_wait: Duration,
    socket_timeout: Duration,

    ip_address: SocketAddr,

    sensors: Sensors,
}

#[derive(Debug)]
pub struct Sensors {
    // Holds ids, battery status and signal level
    parsers: HashMap<u8, ParseInfo<'static>>,
}

#[derive(Debug, Clone)]
pub struct SensorMetadata {
    pub type_id: u8,
    pub type_id_str: String,
    pub type_desc: String,
    pub address: u32,
    pub battery_level: Option<f64>,
    pub battery_state: Option<SensorBatteryState>,
    pub signal: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum SensorBatteryState {
    Ok,
    Low,
    Connected,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct SensorData {
    field: String,
    value: SensorValue,
}

#[derive(Debug, Copy, Clone)]
pub enum SensorValue {
    Empty,
    Temp(f64),
    Humidity(f64),
    Pressure(f64),
    Speed(f64),
    Rain(f64),
    RainLarge(f64),
    Distance(i8),
    Direction(i16),
    UtcTime(i32),
    Count(u32),
    Gain(f64),
    DateTime([u8; 6]),
    Pm10(f64),
    Pm25(f64),
    Co2(i16),
    Light(f64),
    Uv(f64),
    UvIndex(f64),
    Leak(f64),
    Moist(f64),
    Battery(SensorBatteryState),
}

#[derive(Debug)]
struct ParseInfo<'a> {
    parse_fn: fn(&[u8]) -> Result<Vec<SensorValue>, String>,
    field_names: Vec<&'a str>,
    size: usize,
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum GatewayCommands {
    ReadStationMac = 0x26,
    LiveData = 0x27,
    ReadSensorIdNew = 0x3c,
    ReadFirmwareVersion = 0x50,
}

impl SensorData {
    pub fn new(field: &str, value: SensorValue) -> Self {
        SensorData {
            field : field.to_string(),
            value: value.clone(),
        }
    }

    pub fn value(&self) -> &SensorValue {
        &self.value
    }

    pub fn name(&self) -> &str {
        return self.field.as_str();
    }
}

impl SensorGateway {
    pub fn new(ip_address: String, port: u16) -> Self {
        let mut gateway = SensorGateway {
            ip_address: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(Ipv4Addr::from_str(&ip_address).unwrap(), port)),
            max_tries: 3,
            retry_wait: Duration::from_secs(2),
            socket_timeout: Duration::from_secs(2),
            sensors: Sensors::new(),
            firmware: Option::None,
            mac_address: Option::None,
        };

        let _not_used = gateway.update_sensor_metadata();
        if let Ok(firmware) = gateway.get_firmware_version() {
            gateway.firmware = Some(firmware);
        }
        
        if let Ok(mac_address) = gateway.get_station_mac() {
            gateway.mac_address = Some(mac_address);
        }

        gateway
    }

    pub fn name(&self) -> String {
        let mut name = self.mac_address.clone().unwrap().replace(":", "").to_lowercase();
        if cfg!(debug_assertions) {
            // Make it possible to run both release and debug at the same time
            name = name + "_debug";
        }

        return name;
    }

    pub fn version(&self) -> String {
        // TODO: something smarter?
        return self.firmware.clone().unwrap().replace(":", "");
    }

    pub fn firmware(&self) -> String {
        return self.firmware.clone().unwrap().replace(":", "");
    }

    fn generate_checksum(data: &[u8]) -> u8 {
        let mut checksum = 0u8;
        for &byte in data.iter() {
            checksum = checksum.wrapping_add(byte);
        }
        checksum
    }

    fn validate_response(response: &[u8], command: &u8) -> Result<(), String> {
        if response.get(2) == Some(command) {
            let checksum = Self::generate_checksum(&response[2..response.len() - 1]);
            let resp_checksum = *response.last().unwrap_or(&0);
            
            if checksum == resp_checksum {
                Ok(())
            } else {
                Err(format!("Invalid checksum in API response. Expected '{}' (0x{:02X}), received '{}' (0x{:02X}).", 
                            checksum, checksum, resp_checksum, resp_checksum))
            }
        } else {
            let resp_int = response.get(2).cloned().unwrap_or(0);  // Assuming a default value of 0 if response is too short, you can adjust as needed
            Err(format!("Invalid command code in API response. Expected '{}' (0x{:02X}), received '{}' (0x{:02X}).", 
                        command, command, resp_int, resp_int))
        }
    }

    fn bytes_to_hex(data: &[u8], separator: &str) -> String {
        data.iter().map(|byte| format!("{:02X}", byte)).collect::<Vec<_>>().join(separator)
    }

    fn build_cmd_packet(&self, cmd: &GatewayCommands, payload: &[u8]) -> Vec<u8> {
        let size = payload.len() as u8 + 3; // cmd+size+checksum

        let mut body = Vec::new();
        body.push(*cmd as u8);
        body.push(size);
        body.extend_from_slice(payload);

        let checksum = SensorGateway::generate_checksum(&body);

        let mut packet = Vec::new();
        packet.extend_from_slice(&HEADER);
        packet.append(&mut body);
        packet.push(checksum);

        packet
    }

    fn connect_and_send_packet(&self, packet: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        let mut s: TcpStream = TcpStream::connect_timeout(&self.ip_address, self.socket_timeout)?;

        s.set_read_timeout(Some(self.socket_timeout))?;
        s.set_write_timeout(Some(self.socket_timeout))?;

        log::trace!("Sending packet {:?} to {:?}", packet, &self.ip_address);

        // Send the packet.
        s.write_all(packet)?;

        let mut rx_bytes = [0u8; 1024];
        let mut vec = Vec::new();
        let result = s.read(&mut rx_bytes);

        // TODO: check if we need to read more
        match result {
            Ok(n) => {
                vec.extend_from_slice(&rx_bytes[0..n]);
                log::trace!("Received packet {:?} of size {:?} from {:?}", vec, n, &self.ip_address);
            },
            Err(error) => {
                log::error!("Failed to receive packet from {:?} - error {:?}, original packet {:?}", &self.ip_address, error, packet);
            }
        }

        let res = s.shutdown(std::net::Shutdown::Both);
        if res.is_err() {
            log::error!("Failed to shutdown connection to {:?}", &self.ip_address);
        }

        Ok(vec)
    }

    fn send_cmd(&self, cmd: &GatewayCommands, payload: &[u8]) -> Result<Vec<u8>, Error> {
        let mut response: Vec<u8>;

        for attempt in 0..self.max_tries {
            // Construct the message packet.
            let packet = self.build_cmd_packet(cmd, payload);

            // Wrap in a `while` loop to handle retries.
            match self.connect_and_send_packet(&packet) {
                Ok(data) => response = data,
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // A socket timeout occurred, log it.
                    println!("Failed to obtain response to attempt {} to send command '{:?}': {}", attempt + 1, cmd, e);
                    continue;
                }
                Err(ref e) => {
                    // An exception was encountered, log it.
                    println!("Failed attempt {} to send command '{:?}': {}", attempt + 1, cmd, e);
                    continue;
                }
            }

            // Check if the response is valid.
            match SensorGateway::validate_response( &response, &(*cmd as u8)) {
                Ok(_) => return Ok(response),
                Err(ref e) => {
                    // Some other error occurred in check_response(), perhaps the response was malformed.
                    // Log the error and continue.
                    println!("Unexpected exception occurred while checking response to attempt {} to send command '{:?}': {}", attempt + 1, cmd, e);
                }
            }

            // Sleep before our next attempt, but skip the sleep if we have just made our last attempt.
            if attempt < self.max_tries - 1 {
                log::trace!("Retry {:?} sleeping for {:?}", attempt, self.retry_wait);
                sleep(self.retry_wait);
            }
        }

        return Err(Error::new(ErrorKind::Other, format!("Failed to obtain response to command '{:?}' after {} attempts", cmd, self.max_tries)));
    }

    fn parse_live_data(&self, response: &[u8]) -> Result<Vec<Vec<SensorData>>, String> {
        // Obtain the payload size as a big-endian unsigned short
        let payload_size = u16::from_be_bytes([response[3], response[4]]) as usize;

        // Check if the response has enough data for the payload
        if response.len() < payload_size {
            return Err(format!("Payload size does not match response length len: {:?} payload:{:?}", response.len(), payload_size + 5));
        }

        let val = self.sensors.parse_live_data(&response[5..5 + payload_size - 4]);
        val
    }

    pub fn update_sensor_metadata(&self) -> Result<HashMap<u32, SensorMetadata>, String> {
        let sensor_ids = self.send_cmd(&GatewayCommands::ReadSensorIdNew, &[]);
        match sensor_ids {
            Ok(data) => {
                self.sensors.update_metadata(&data)
            },
            Err(err) => {
                log::error!("Failed to parse sensor metadata - {:?}", err);
                Err(format!("Failed to parse sensor metadata - {:?}", err))
            }
        }
    }

    pub fn get_live_data(&self) -> Result<Vec<Vec<SensorData>>, String> {
        let live_data = self.send_cmd(&GatewayCommands::LiveData, &[]);
        match live_data {
            Ok(data) => {
                self.parse_live_data(&data)
            }
            Err(err) => {
                log::error!("Failed to parse sensor live data - {:?}", err);
                Err(format!("Failed to parse sensor live data - {:?}", err))
            }
        }
    }

    pub fn get_firmware_version(&mut self) -> Result<String, String> {
        let firmware_data = self.send_cmd(&GatewayCommands::ReadFirmwareVersion,&[]);
        match firmware_data {
            Ok(data) => {
                let fw_size = data[4] as usize;
                let fw_bytes = &data[5..5 + fw_size];
                match String::from_utf8(fw_bytes.to_vec()) {
                    Ok(s) => return Ok(s),
                    Err(_) => return Err(format!("Invalid UTF-8 sequence {:?}", fw_bytes)),
                };
            }
            Err(err) => {
                log::error!("Failed to parse firmware version - {:?}", err);
                Err(format!("Failed to parse firmware version - {:?}", err))
            }
        }
    }

    pub fn get_station_mac(&mut self) -> Result<String, String> {
        let mac = self.send_cmd(&GatewayCommands::ReadStationMac,&[]);
        match mac {
            Ok(data) => {
                Ok(SensorGateway::bytes_to_hex(&data[3..10], ":"))
            }
            Err(err) => {
                log::error!("Failed to parse firmware version - {:?}", err);
                Err(format!("Failed to parse firmware version - {:?}", err))
            }
        }
    }
}

impl SensorValue {
    fn round(x: &f64) -> f64 {
        (x * 100.0).round() / 100.0
    }

    pub fn to_json_val(t: &SensorValue) -> Value {
        log::trace!("to_json_val: {:?}", t);
        match t {
            SensorValue::Empty => json!(null),
            SensorValue::Temp(val) => json!(Self::round(val)),
            SensorValue::Humidity(val) => json!(Self::round(val)),
            SensorValue::Pressure(val) => json!(Self::round(val)),
            SensorValue::Speed(val) => json!(Self::round(val)),
            SensorValue::Rain(val) => json!(Self::round(val)),
            SensorValue::RainLarge(val) => json!(Self::round(val)),
            SensorValue::Distance(val) => json!(val),
            SensorValue::Direction(val) => json!(val),
            SensorValue::UtcTime(val) => json!(val),
            SensorValue::Count(val) => json!(val),
            SensorValue::Gain(val) => json!(Self::round(val)),
            SensorValue::DateTime(val) => json!(format!("dt:{:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",val[0],val[1],val[2],val[3],val[4],val[5])),
            SensorValue::Pm10(val) => json!(Self::round(val)),
            SensorValue::Pm25(val) => json!(Self::round(val)),
            SensorValue::Co2(val) => json!(val),
            SensorValue::Light(val) => json!(Self::round(val)),
            SensorValue::Uv(val) => json!(Self::round(val)),
            SensorValue::UvIndex(val) => json!(Self::round(val)),
            SensorValue::Leak(val) => json!(Self::round(val)),
            SensorValue::Battery(val) => {
                let str = match val {
                    SensorBatteryState::Ok => "ok",
                    SensorBatteryState::Low => "low",
                    SensorBatteryState::Connected => "connected",
                    SensorBatteryState::Unknown => "unknown",                    
                };
                json!(str)
            },
            SensorValue::Moist(val) => json!(Self::round(val)),
        }
    }

    pub fn parse_temp(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 2 { return Err("Invalid data length temp".to_string()); }
        Ok(vec![SensorValue::Temp(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as f64 / 10.0)])
    }

    pub fn parse_humidity(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 1 { return Err("Invalid data length for humidity".to_string()); }
        Ok(vec![SensorValue::Humidity(data[0] as f64)])
    }

    pub fn parse_moist(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 1 { return Err("Invalid data length for moist".to_string()); }
        Ok(vec![SensorValue::Moist(data[0] as f64)])
    }

    pub fn parse_pressure(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() < 2 { return Err("Invalid data length for pressure".to_string()); }
        Ok(vec![SensorValue::Pressure(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as f64 / 10.0)])
    }
    
    pub fn parse_speed(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() < 2 { return Err("Invalid data length for speed".to_string()); }
        Ok(vec![SensorValue::Speed(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as f64 / 10.0)])
    }

    pub fn parse_rain(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() < 2 { return Err("Invalid data length for rain".to_string()); }
        Ok(vec![SensorValue::Rain(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as f64 / 10.0)])
    }

    pub fn parse_rainlarge(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 4 { return Err("Invalid data length for rainlarge".to_string()); }
        Ok(vec![SensorValue::RainLarge(u32::from_be_bytes(data.try_into().unwrap()) as f64 / 10.0)])
    }

    pub fn parse_distance(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 1 { return Err("Invalid data length for humidity".to_string()); }
        Ok(vec![SensorValue::Distance(data[0] as i8)])
    }

    pub fn parse_direction(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 2 { return Err("Invalid data lenght for direction".to_string()); }
        Ok(vec![SensorValue::Direction(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()))])
    }

    pub fn parse_count(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 4 { return Err("Invalid data length for count".to_string()); }
        Ok(vec![SensorValue::Count(u32::from_be_bytes(data.try_into().unwrap()))])
    }

    pub fn parse_gain(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 4 { return Err("Invalid data length for gain".to_string()); }
        Ok(vec![SensorValue::Gain(u32::from_be_bytes(data.try_into().unwrap()) as f64 / 100.0)])
    }

    pub fn parse_light(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 4 { return Err("Invalid data length for light".to_string()); }
        Ok(vec![SensorValue::Light(u32::from_be_bytes(data.try_into().unwrap()) as f64 / 100.0)])
    }

    pub fn parse_uv(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() < 2 { return Err("Invalid data length for uv".to_string()); }
        Ok(vec![SensorValue::Uv(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as f64 / 10.0)])
    }

    pub fn parse_uv_index(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 1 { return Err("Invalid data length for uv index".to_string()); }
        Ok(vec![SensorValue::UvIndex(data[0] as f64)])
    }

    pub fn parse_pm10(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() < 2 { return Err("Invalid data length for pm10".to_string()); }
        Ok(vec![SensorValue::Pm10(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as f64 / 10.0)])
    }

    pub fn parse_pm25(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() < 2 { return Err("Invalid data length for pm25".to_string()); }
        Ok(vec![SensorValue::Pm25(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as f64 / 10.0)])
    }

    pub fn parse_leak(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 1 { return Err("Invalid data length for leak".to_string()); }
        Ok(vec![SensorValue::Leak(data[0] as f64)])
    }

    pub fn parse_co2(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 2 { return Err("Invalid data lenght for co2".to_string()); }
        Ok(vec![SensorValue::Co2(i16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()))])
    }

    pub fn parse_utc(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 4 { return Err("Invalid data length for utc time".to_string()); }
        let utc = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        Ok(vec![SensorValue::UtcTime(utc)])
    }

    pub fn parse_datetime(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 6 { return Err("Invalid data length for utc time".to_string()); }
        Ok(vec![SensorValue::DateTime(data.try_into().unwrap())])
    }

    pub fn parse_wh45(data: &[u8]) -> Result<Vec<SensorValue>, String> {
        if data.len() != 6 { return Err("Invalid data length for wh45 sensor".to_string()); }
    
        let temp = Self::parse_temp(&data[0..2]).unwrap()[0];
        let humid = Self::parse_humidity(&[data[2]]).unwrap()[0];
        let pm10 = Self::parse_pm10(&data[3..5]).unwrap()[0];
        let pm10_avg = Self::parse_humidity(&data[5..7]).unwrap()[0];
        let pm25 = Self::parse_pm25(&data[7..9]).unwrap()[0];
        let pm25_avg = Self::parse_pm25(&data[9..11]).unwrap()[0];
        let co2 = Self::parse_co2(&data[11..13]).unwrap()[0];
        let co2_avg = Self::parse_co2(&data[13..15]).unwrap()[0];
        // TODO: do we need to parse battery state here

        Ok(vec![temp, humid, pm10, pm10_avg, pm25, pm25_avg, co2, co2_avg])
    }

    pub fn skip_data(_data: &[u8]) -> Result<Vec<SensorValue>, String> {
        Ok(vec![SensorValue::Empty])
    }

}

impl SensorMetadata {
    fn new(type_id: u8, address: u32, battery: Option<f64>, signal: u8) -> Self {
        let type_id_str = Self::parse_type(type_id).unwrap_or("unknown".to_string());
        let type_desc = Self::parse_type_desc(type_id).unwrap_or("unknown".to_string());
        let battery_state = Self::parse_battery_state(type_id, battery.unwrap());

        SensorMetadata { type_id, type_desc, type_id_str, address, battery_level : battery, battery_state, signal }
    }

    fn parse_type(id: u8) -> Option<String> {
        match id {
            0x0 => Some("wh65".to_string()),
            0x1 => Some("wh68".to_string()),
            0x2 => Some("wh80".to_string()),
            0x3 => Some("wh40".to_string()),
            0x4 => Some("wh25".to_string()),
            0x5 => Some("wh26".to_string()),
            0x6..=0xd => Some(format!("wh31_ch{:?}", id - 5)),
            0xe..=0x15 => Some(format!("wh51_ch{:?}", id - 0xd)),
            0x16..=0x19 => Some(format!("wh41_ch{:?}", id - 0x15)),
            0x1a => Some("wh57".to_string()),
            0x1b..=0x1e => Some(format!("wh55_ch{:?}", id - 0x1a)),
            0x1f..=0x25 => Some(format!("wh34_ch{:?}", id - 0x1e)),
            0x27 => Some("wh45".to_string()),
            0x28..=0x2f => Some(format!("wh35_ch{:?}", id - 0x27)),
            _ => None
        }
    }

    fn parse_type_desc(id: u8) -> Option<String> {
        match id {
            0x0 => Some("WH-65".to_string()),
            0x1 => Some("WH-68".to_string()),
            0x2 => Some("WH-80".to_string()),
            0x3 => Some("WH-40".to_string()),
            0x4 => Some("WH-25".to_string()),
            0x5 => Some("WH-26".to_string()),
            0x6..=0xd => Some(format!("WH-31 channel {:?}", id - 5)),
            0xe..=0x15 => Some(format!("WH-51 channel {:?}", id - 0xd)),
            0x16..=0x19 => Some(format!("WH-41 channel {:?}", id - 0x15)),
            0x1a => Some("WH-57".to_string()),
            0x1b..=0x1e => Some(format!("WH-55 channel {:?}", id - 0x1a)),
            0x1f..=0x25 => Some(format!("WH-34 channel {:?}", id - 0x1e)),
            0x27 => Some("WH-45".to_string()),
            0x28..=0x2f => Some(format!("WH-35 channel {:?}", id - 0x27)),
            _ => None
        }
    }

    fn parse_battery_state(id: u8, battery: f64) -> Option<SensorBatteryState> {
        match id {
            0|4|5..=0xd => {
                log::trace!("Binary battery: id {:#x?} {:?} volt", id, battery);
                // Binary
                if battery == 1.0 {
                    Some(SensorBatteryState::Low) 
                } else if battery == 0.0 { 
                    Some(SensorBatteryState::Ok) 
                } else { 
                    Some(SensorBatteryState::Unknown) 
                }
            }
            0x16..=0x1e|0x27 => {
                log::trace!("Integer battery: id {:#x?} {:?} volt", id, battery);
                // Integer
                if battery <= 1.0 { 
                    Some(SensorBatteryState::Low) 
                } else if battery <= 5.0 {
                    Some(SensorBatteryState::Ok) 
                } else if battery == 6.0 {
                    Some(SensorBatteryState::Connected) 
                } else { 
                    Some(SensorBatteryState::Unknown) 
                }
            }
            1..=3|0xe..=0x15|0x1f..=0x26|0x28..=0x30 => {
                log::trace!("Volt battery: id {:#x?} {:?} volt", id, battery);
                // Volt
                if battery <= 1.2 { 
                    Some(SensorBatteryState::Low) 
                } else { 
                    Some(SensorBatteryState::Ok) 
                }
            }
            _ => Some(SensorBatteryState::Unknown)
        }
    } 
}

impl Sensors {
    pub fn new() -> Self {
        Sensors {
            parsers : Self::init_parsers(),
        }
    }

    fn init_parsers() -> HashMap<u8, ParseInfo<'static>> {
        let mut parsers: HashMap<u8, ParseInfo<'static>> = HashMap::new();

        parsers.insert(0x01, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["indoor_temp"], size: 2});
        parsers.insert(0x02, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["outdoor_temp"], size: 2});
        parsers.insert(0x04, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["windchill"], size: 2});
        parsers.insert(0x05, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["heat_index"], size: 2});
        parsers.insert(0x06, ParseInfo { parse_fn: SensorValue::parse_humidity, field_names: vec!["in_humidity"], size: 1});
        parsers.insert(0x07, ParseInfo { parse_fn: SensorValue::parse_humidity, field_names: vec!["out_humidity"], size: 1});
        parsers.insert(0x08, ParseInfo { parse_fn: SensorValue::parse_pressure, field_names: vec!["abs_barometer"], size: 2});
        parsers.insert(0x09, ParseInfo { parse_fn: SensorValue::parse_pressure, field_names: vec!["rel_barometer"], size: 2});
        parsers.insert(0x0A, ParseInfo { parse_fn: SensorValue::parse_direction, field_names: vec!["wind_dir"], size: 2});
        parsers.insert(0x0B, ParseInfo { parse_fn: SensorValue::parse_speed, field_names: vec!["wind_speed"], size: 2});
        parsers.insert(0x0C, ParseInfo { parse_fn: SensorValue::parse_speed, field_names: vec!["gust_speed"], size: 2});
        parsers.insert(0x0D, ParseInfo { parse_fn: SensorValue::parse_rain, field_names: vec!["rain_event"], size: 2});
        // TODO: rain rate special?
        parsers.insert(0x0E, ParseInfo { parse_fn: SensorValue::parse_rain, field_names: vec!["rain_rate"], size: 2});
        parsers.insert(0x0F, ParseInfo { parse_fn: SensorValue::parse_gain, field_names: vec!["rain_gain"], size: 2});
        parsers.insert(0x10, ParseInfo { parse_fn: SensorValue::parse_rain, field_names: vec!["rain_day"], size: 2});
        parsers.insert(0x11, ParseInfo { parse_fn: SensorValue::parse_rain, field_names: vec!["rain_week"], size: 2});
        parsers.insert(0x12, ParseInfo { parse_fn: SensorValue::parse_rainlarge, field_names: vec!["rain_month"], size: 4});
        parsers.insert(0x13, ParseInfo { parse_fn: SensorValue::parse_rainlarge, field_names: vec!["rain_year"], size: 4});
        parsers.insert(0x14, ParseInfo { parse_fn: SensorValue::parse_rainlarge, field_names: vec!["rain_totals"], size: 4});
        parsers.insert(0x15, ParseInfo { parse_fn: SensorValue::parse_light, field_names: vec!["light"], size: 4});
        parsers.insert(0x16, ParseInfo { parse_fn: SensorValue::parse_uv, field_names: vec!["uv"], size: 2});
        parsers.insert(0x17, ParseInfo { parse_fn: SensorValue::parse_uv_index, field_names: vec!["uv_index"], size: 1});
        parsers.insert(0x18, ParseInfo { parse_fn: SensorValue::parse_datetime, field_names: vec!["datetime"], size: 6});
        parsers.insert(0x19, ParseInfo { parse_fn: SensorValue::parse_speed, field_names: vec!["day_maxwind"], size: 2});
        parsers.insert(0x1A, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_1"], size: 2});
        parsers.insert(0x1B, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_2"], size: 2});
        parsers.insert(0x1C, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_3"], size: 2});
        parsers.insert(0x1D, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_4"], size: 2});
        parsers.insert(0x1E, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_5"], size: 2});
        parsers.insert(0x1F, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_6"], size: 2});
        parsers.insert(0x20, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_7"], size: 2});
        parsers.insert(0x21, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["temp_8"], size: 2});
        parsers.insert(0x22, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_1"], size: 1});
        parsers.insert(0x23, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_2"], size: 1});
        parsers.insert(0x24, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_3"], size: 1});
        parsers.insert(0x25, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_4"], size: 1});
        parsers.insert(0x26, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_5"], size: 1});
        parsers.insert(0x27, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_6"], size: 1});
        parsers.insert(0x28, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_7"], size: 1});
        parsers.insert(0x29, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["humidity_8"], size: 1});
        parsers.insert(0x2A, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_1"], size: 2});
        
        parsers.insert(0x2B, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_1"], size: 2});
        parsers.insert(0x2C, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_1"], size: 1});
        parsers.insert(0x2D, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_2"], size: 2});
        parsers.insert(0x2E, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_2"], size: 1});
        parsers.insert(0x2F, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_3"], size: 2});
        parsers.insert(0x30, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_3"], size: 1});
        parsers.insert(0x31, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_4"], size: 2});
        parsers.insert(0x32, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_4"], size: 1});
        parsers.insert(0x33, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_5"], size: 2});
        parsers.insert(0x34, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_5"], size: 1});
        parsers.insert(0x35, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_6"], size: 2});
        parsers.insert(0x36, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_6"], size: 1});
        parsers.insert(0x37, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_7"], size: 2});
        parsers.insert(0x38, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_7"], size: 1});
        parsers.insert(0x39, ParseInfo { parse_fn: SensorValue::parse_temp, field_names: vec!["soil_temp_8"], size: 2});
        parsers.insert(0x3A, ParseInfo { parse_fn: SensorValue::parse_moist, field_names: vec!["soil_moist_8"], size: 1});

        // Skip old battery info (old firmware)
        parsers.insert(0x4C, ParseInfo { parse_fn: SensorValue::skip_data, field_names: vec![""], size: 16});

        parsers.insert(0x4D, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_1_avg_24h"], size: 2});
        parsers.insert(0x4E, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_2_avg_24h"], size: 2});
        parsers.insert(0x4F, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_3_avg_24h"], size: 2});
        parsers.insert(0x50, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_4_avg_24h"], size: 2});

        parsers.insert(0x51, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_2"], size: 2});
        parsers.insert(0x52, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_3"], size: 2});
        parsers.insert(0x53, ParseInfo { parse_fn: SensorValue::parse_pm25, field_names: vec!["pm25_4"], size: 2});

        parsers.insert(0x58, ParseInfo { parse_fn: SensorValue::parse_leak, field_names: vec!["leak1"], size: 1});
        parsers.insert(0x59, ParseInfo { parse_fn: SensorValue::parse_leak, field_names: vec!["leak2"], size: 1});
        parsers.insert(0x5A, ParseInfo { parse_fn: SensorValue::parse_leak, field_names: vec!["leak3"], size: 1});
        parsers.insert(0x5B, ParseInfo { parse_fn: SensorValue::parse_leak, field_names: vec!["leak4"], size: 1});

        parsers.insert(0x60, ParseInfo { parse_fn: SensorValue::parse_distance, field_names: vec!["lightning_distance"], size: 1});
        parsers.insert(0x61, ParseInfo { parse_fn: SensorValue::parse_utc, field_names: vec!["lightning_datetime"], size: 4});
        parsers.insert(0x62, ParseInfo { parse_fn: SensorValue::parse_count, field_names: vec!["lightning_count"], size: 4});

        parsers.insert(0x70, ParseInfo { parse_fn: SensorValue::parse_wh45, field_names: vec!["temp_wh45", "humid_wh45", "pm10_wh45", "pm10_avg_24h_wh45", "pm25_wh45", "pm25_avg_24h_wh45", "co2_wh45", "co2_avg_24h_wh45"], size:16});

        parsers
    }

    pub fn update_metadata(&self, id_data: &[u8]) -> Result<HashMap<u32, SensorMetadata>, String> {
        let mut metadata = HashMap::new();
        if !id_data.is_empty() {
            let data_size_bytes: [u8; 2] = id_data[3..5].try_into().expect("Failed to convert data to array");
            let data_size = u16::from_be_bytes(data_size_bytes);

            // Extract the actual sensor ID data.
            let data = &id_data[5..(5 + data_size as usize - 4)];

            // Initialize a counter.
            let mut index = 0;

            // Iterate over the data.
            while index < data.len() {
                let type_id: u8 = data[index];
                let sensor_id_bytes: [u8; 4] = data[(index + 1)..(index + 5)]
                        .try_into()
                        .expect("Failed to convert sensor ID bytes");
                
                let address = u32::from_be_bytes(sensor_id_bytes);
                let batt = data[index + 5];
                let signal = data[index + 6];

                log::trace!("Metadata type={} address:{} battery:{} signal:{}", type_id, address, batt, signal);

                // check if the sensor is active or not
                if address != 0xffffffff {
                    let meta = SensorMetadata::new(type_id, address, Some(f64::from(batt)), signal);
                    log::debug!("Meta={:?}", meta);
                    if meta.type_id_str.eq("unknown") {
                        log::warn!("Found unknown sensor {:?}", meta);
                    }
                    
                    metadata.insert(address, meta);
                }

                // Each sensor entry is seven bytes in length, so skip to the start of the next sensor.
                index += 7;
            }
        } 
        Ok(metadata)
    }

    pub fn parse_live_data(&self, data: &[u8]) -> Result<Vec<Vec<SensorData>>, String> {
        let mut sensor_data: Vec<Vec<SensorData>> = Vec::new();

        let mut index = 0;
        while index < data.len() {
            // type_id = type of sensor, lookup parser and parse
            let type_id = data[index];
            if let Some(&parser) = self.parsers.get(&type_id).as_ref() {
                log::trace!("Found type {:#x}", type_id);
                if index + 1 + parser.size <= data.len() {
                    // Some sensors can have multiple fields/values, hard coded order
                    // in the parser setup
                    let field_data = data[index + 1..index + 1 + parser.size].to_vec();
                    if let Ok(parsed_data) = (parser.parse_fn)(&field_data) {
                        let mut values = Vec::new();
                        for i in 0..parsed_data.len() {
                            let val = parsed_data[i];
                            let name = parser.field_names[i];

                            log::trace!("field: {:?} val:{:?}", name, val);
                            values.push(SensorData::new(name, val));
                        }

                        sensor_data.push(values);
                    }               
                }
                index += parser.size as usize + 1;
            } else {
                return Err(format!("Failed to find parser for type id {:#x}", type_id));
            }
        }

        Ok(sensor_data)
    }
}
