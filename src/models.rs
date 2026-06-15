use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::io::Read;
use crate::constants::{NODATA, RAIN_THRESHOLD};

#[derive(Clone, Debug)]
pub struct LutEntry {
    pub indices: [u32; 4],
    pub weights: [f32; 4],
}

/// Metadata describing the current NetCDF dataset geometry and contents.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Metadata {
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
    pub top: f64,
    pub width: u32,
    pub height: u32,
    pub ensembles: Vec<i32>,
    pub times: Vec<i64>,
    pub reference_time_str: String,
    pub version: u64,
}

pub struct TempStep {
    pub forecast_hour: i32,
    pub width: usize,
    pub height: usize,
    pub values: Arc<Vec<u16>>,
}

pub struct TempForecast {
    pub reference_time: i64,
    pub steps: Vec<TempStep>,
}

impl TempForecast {
    pub fn write_to_file(&self, path: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        f.write_all(b"HRMT")?;
        f.write_all(&self.reference_time.to_le_bytes())?;
        f.write_all(&(self.steps.len() as u32).to_le_bytes())?;
        
        for step in &self.steps {
            f.write_all(&step.forecast_hour.to_le_bytes())?;
            f.write_all(&(step.width as u32).to_le_bytes())?;
            f.write_all(&(step.height as u32).to_le_bytes())?;
            for &val in step.values.as_ref() {
                f.write_all(&val.to_le_bytes())?;
            }
        }
        f.flush()?;
        Ok(())
    }

    pub fn read_from_file(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut f = std::fs::File::open(path)?;
        let mut magic = [0u8; 4];
        f.read_exact(&mut magic)?;
        if &magic != b"HRMT" {
            return Err("Invalid magic bytes in temp file".into());
        }
        
        let mut ref_time_bytes = [0u8; 8];
        f.read_exact(&mut ref_time_bytes)?;
        let reference_time = i64::from_le_bytes(ref_time_bytes);
        
        let mut steps_len_bytes = [0u8; 4];
        f.read_exact(&mut steps_len_bytes)?;
        let steps_len = u32::from_le_bytes(steps_len_bytes) as usize;
        
        let mut steps = Vec::with_capacity(steps_len);
        for _ in 0..steps_len {
            let mut hour_bytes = [0u8; 4];
            f.read_exact(&mut hour_bytes)?;
            let forecast_hour = i32::from_le_bytes(hour_bytes);
            
            let mut w_bytes = [0u8; 4];
            f.read_exact(&mut w_bytes)?;
            let width = u32::from_le_bytes(w_bytes) as usize;
            
            let mut h_bytes = [0u8; 4];
            f.read_exact(&mut h_bytes)?;
            let height = u32::from_le_bytes(h_bytes) as usize;
            
            let len = width * height;
            let mut values = vec![0u16; len];
            let mut byte_buf = vec![0u8; len * 2];
            f.read_exact(&mut byte_buf)?;
            for i in 0..len {
                values[i] = u16::from_le_bytes([byte_buf[i * 2], byte_buf[i * 2 + 1]]);
            }
            
            steps.push(TempStep {
                forecast_hour,
                width,
                height,
                values: Arc::new(values),
            });
        }
        
        Ok(TempForecast {
            reference_time,
            steps,
        })
    }
}

pub struct WindStep {
    pub forecast_hour: i32,
    pub width: usize,
    pub height: usize,
    pub u_values: Arc<Vec<u16>>,
    pub v_values: Arc<Vec<u16>>,
}

pub struct WindForecast {
    pub reference_time: i64,
    pub steps: Vec<WindStep>,
}

impl WindForecast {
    pub fn write_to_file(&self, path: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        f.write_all(b"HRMW")?;
        f.write_all(&self.reference_time.to_le_bytes())?;
        f.write_all(&(self.steps.len() as u32).to_le_bytes())?;
        
        for step in &self.steps {
            f.write_all(&step.forecast_hour.to_le_bytes())?;
            f.write_all(&(step.width as u32).to_le_bytes())?;
            f.write_all(&(step.height as u32).to_le_bytes())?;
            for &val in step.u_values.as_ref() {
                f.write_all(&val.to_le_bytes())?;
            }
            for &val in step.v_values.as_ref() {
                f.write_all(&val.to_le_bytes())?;
            }
        }
        f.flush()?;
        Ok(())
    }

    pub fn read_from_file(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut f = std::fs::File::open(path)?;
        let mut magic = [0u8; 4];
        f.read_exact(&mut magic)?;
        if &magic != b"HRMW" {
            return Err("Invalid magic bytes in wind file".into());
        }
        
        let mut ref_time_bytes = [0u8; 8];
        f.read_exact(&mut ref_time_bytes)?;
        let reference_time = i64::from_le_bytes(ref_time_bytes);
        
        let mut steps_len_bytes = [0u8; 4];
        f.read_exact(&mut steps_len_bytes)?;
        let steps_len = u32::from_le_bytes(steps_len_bytes) as usize;
        
        let mut steps = Vec::with_capacity(steps_len);
        for _ in 0..steps_len {
            let mut hour_bytes = [0u8; 4];
            f.read_exact(&mut hour_bytes)?;
            let forecast_hour = i32::from_le_bytes(hour_bytes);
            
            let mut w_bytes = [0u8; 4];
            f.read_exact(&mut w_bytes)?;
            let width = u32::from_le_bytes(w_bytes) as usize;
            
            let mut h_bytes = [0u8; 4];
            f.read_exact(&mut h_bytes)?;
            let height = u32::from_le_bytes(h_bytes) as usize;
            
            let len = width * height;
            let mut u_values = vec![0u16; len];
            let mut byte_buf = vec![0u8; len * 2];
            f.read_exact(&mut byte_buf)?;
            for i in 0..len {
                u_values[i] = u16::from_le_bytes([byte_buf[i * 2], byte_buf[i * 2 + 1]]);
            }
            
            let mut v_values = vec![0u16; len];
            f.read_exact(&mut byte_buf)?;
            for i in 0..len {
                v_values[i] = u16::from_le_bytes([byte_buf[i * 2], byte_buf[i * 2 + 1]]);
            }
            
            steps.push(WindStep {
                forecast_hour,
                width,
                height,
                u_values: Arc::new(u_values),
                v_values: Arc::new(v_values),
            });
        }
        
        Ok(WindForecast {
            reference_time,
            steps,
        })
    }
}

/// Query parameters for the `/api/value` endpoint.
#[derive(Deserialize)]
pub struct ValueQuery {
    pub ens: String,
    pub time: i64,
    pub lat: f64,
    pub lon: f64,
}

/// JSON response returned by the `/api/value` endpoint.
#[derive(Serialize)]
pub struct ValueResponse {
    pub status: String,
    pub value: Option<f64>,
}

/// Query parameters for the `/api/timeseries` endpoint.
#[derive(Deserialize)]
pub struct TimeseriesQuery {
    pub ens: String,
    pub lat: f64,
    pub lon: f64,
}

/// JSON response returned by the `/api/timeseries` endpoint.
#[derive(Serialize)]
pub struct TimeseriesResponse {
    pub status: String,
    pub lat: f64,
    pub lon: f64,
    pub ens: String,
    pub times: Vec<i64>,
    pub values: Vec<f64>,
}

/// Deserialized response from the KNMI Open Data download-URL endpoint.
#[derive(Deserialize)]
pub struct FileUrlResponse {
    #[serde(rename = "temporaryDownloadUrl")]
    pub temporary_download_url: String,
}

/// The statistical reduction to apply across ensemble members.
pub enum EnsembleStat {
    /// Take the median member value.
    Median,
    /// Take the maximum member value.
    Maximum,
    /// Compute the percentage of members exceeding [`RAIN_THRESHOLD`].
    Probability,
}

impl EnsembleStat {
    /// Parse a short string identifier into an [`EnsembleStat`].
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "med" => Some(Self::Median),
            "max" => Some(Self::Maximum),
            "prob" => Some(Self::Probability),
            _ => None,
        }
    }
}

/// Reduces a set of ensemble member values into a single statistic.
///
/// If the first member is [`NODATA`] the entire cell is considered missing and
/// [`NODATA`] is returned. For probability mode the result is a percentage
/// (0–100) rather than a raw precipitation value.
pub fn reduce_ensemble(stat: &EnsembleStat, member_vals: &mut [u16]) -> u16 {
    if member_vals.is_empty() || member_vals[0] == NODATA {
        return NODATA;
    }
    match stat {
        EnsembleStat::Maximum => {
            member_vals
                .iter()
                .copied()
                .filter(|&v| v != NODATA)
                .max()
                .unwrap_or(0)
        }
        EnsembleStat::Probability => {
            let count = member_vals
                .iter()
                .copied()
                .filter(|&v| v != NODATA && v >= RAIN_THRESHOLD)
                .count();
            ((count * 100) / member_vals.len()) as u16
        }
        EnsembleStat::Median => {
            member_vals.sort_unstable();
            member_vals[member_vals.len() / 2]
        }
    }
}

#[derive(Serialize)]
pub struct WindMetadata {
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
    pub top: f64,
    pub width: u32,
    pub height: u32,
    pub times: Vec<i64>,
    pub reference_time: i64,
    pub reference_time_str: String,
    pub version: u64,
}

#[derive(Deserialize)]
pub struct WindValueQuery {
    pub lat: f64,
    pub lon: f64,
    pub time: i64,
}

#[derive(Serialize)]
pub struct WindValueResponse {
    pub status: String,
    pub u: Option<f64>,
    pub v: Option<f64>,
    pub speed: Option<f64>,
    pub direction: Option<f64>,
}

#[derive(Deserialize)]
pub struct WindTimeseriesQuery {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize)]
pub struct WindTimeseriesResponse {
    pub status: String,
    pub lat: f64,
    pub lon: f64,
    pub times: Vec<i64>,
    pub speeds: Vec<f64>,
    pub directions: Vec<f64>,
}

#[derive(Serialize)]
pub struct TempMetadata {
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
    pub top: f64,
    pub width: u32,
    pub height: u32,
    pub times: Vec<i64>,
    pub reference_time: i64,
    pub reference_time_str: String,
    pub version: u64,
}

#[derive(Deserialize)]
pub struct TempValueQuery {
    pub lat: f64,
    pub lon: f64,
    pub time: i64,
}

#[derive(Deserialize)]
pub struct TempTimeseriesQuery {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize)]
pub struct TempTimeseriesResponse {
    pub status: String,
    pub lat: f64,
    pub lon: f64,
    pub times: Vec<i64>,
    pub values: Vec<f64>,
}
