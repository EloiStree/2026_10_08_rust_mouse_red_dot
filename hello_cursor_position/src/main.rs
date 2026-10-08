use std::fs;
use std::process::Command;

#[derive(Debug, Default)]
struct Device {
    name: String,
    handlers: String,
}

fn parse_devices() -> Vec<Device> {
    let content = fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
    let mut devices = Vec::new();
    let mut current = Device::default();

    for line in content.lines().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if !current.name.is_empty() {
                devices.push(std::mem::take(&mut current));
            }
        } else if let Some(rest) = line.strip_prefix("N: Name=") {
            current.name = rest.trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
            current.handlers = rest.to_string();
        }
    }
    devices
}

/// Returns the pointer position (x, y) using xdotool (X11).
fn mouse_position() -> Option<(i32, i32)> {
    let out = Command::new("xdotool")
        .arg("getmouselocation")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut x = None;
    let mut y = None;
    for part in text.split_whitespace() {
        if let Some(v) = part.strip_prefix("x:") {
            x = v.parse().ok();
        } else if let Some(v) = part.strip_prefix("y:") {
            y = v.parse().ok();
        }
    }
    Some((x?, y?))
}

fn main() {
    let devices = parse_devices();

    let mice: Vec<&Device> = devices
        .iter()
        .filter(|d| {
            let n = d.name.to_lowercase();
            d.handlers.split_whitespace().any(|h| h.starts_with("mouse"))
                || n.contains("mouse")
                || n.contains("touchpad")
                || n.contains("trackpad")
                || n.contains("pointer")
        })
        .collect();
    let keyboards: Vec<&Device> = devices
        .iter()
        .filter(|d| d.handlers.split_whitespace().any(|h| h == "kbd"))
        .collect();

    println!("Keyboards ({}):", keyboards.len());
    for (i, k) in keyboards.iter().enumerate() {
        println!("  {}. {}", i + 1, k.name);
    }

    // The system has a single shared pointer position, reported for every mouse.
    loop {
        let pos = mouse_position();
        println!("Mice ({}):", mice.len());
        for (i, m) in mice.iter().enumerate() {
            match pos {
                Some((x, y)) => println!("  {}. {} - position: x={}, y={}", i + 1, m.name, x, y),
                None => println!("  {}. {} - position: unavailable", i + 1, m.name),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
