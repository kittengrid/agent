use super::{BlkioLimit, BlkioWeight};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct BlkioConfig {
    pub device_read_bps: Option<Vec<BlkioLimit>>,
    pub device_read_iops: Option<Vec<BlkioLimit>>,
    pub device_write_bps: Option<Vec<BlkioLimit>>,
    pub device_write_iops: Option<Vec<BlkioLimit>>,
    pub weight: Option<i32>,
    pub weight_device: Option<Vec<BlkioWeight>>,
}
