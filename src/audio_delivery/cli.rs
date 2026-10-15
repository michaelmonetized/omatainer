use super::{measurement, process};
use std::{path::Path, sync::atomic::AtomicBool};

/// Handle device-free delivery diagnostics before native application startup.
/// Args: process arguments without executable. Returns whether a delivery command was handled, or an explicit input/tool error.
pub(crate) fn run(args: &[String]) -> Result<bool, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    let cancel = AtomicBool::new(false);
    let value = match command {
        "delivery-preflight" => {
            if args.len() != 1 {
                return Err("usage: omatainer delivery-preflight".into());
            }
            measurement::preflight(&cancel)?
        }
        "delivery-measure" => {
            if !(4..=5).contains(&args.len()) {
                return Err(
                    "usage: omatainer delivery-measure FILE RATE CHANNELS [DECIMAL_WAVE_MASK]"
                        .into(),
                );
            }
            let rate = args[2].parse().map_err(|_| "Invalid sample rate")?;
            let channels = args[3].parse().map_err(|_| "Invalid channel count")?;
            let mask = args
                .get(4)
                .map(|v| v.parse::<u32>().map_err(|_| "Invalid decimal WAVE mask"))
                .transpose()?;
            let path = Path::new(&args[1]);
            let report = measurement::measure(path, rate, channels, mask, &cancel)?;
            serde_json::json!({"schema":1,"file":path,"file_sha256":process::sha256_cancelled(path,&cancel)?,"qc":report})
        }
        _ => return Ok(false),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(true)
}
