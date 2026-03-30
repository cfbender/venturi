use std::collections::BTreeSet;
use std::process::Command;

pub(crate) fn run_command(program: &str, args: &[String]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run {program}: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

pub(crate) fn run_wpctl(args: &[String]) {
    let _ = run_command("wpctl", args);
}

pub(crate) fn run_wpctl_checked(args: &[String]) -> Result<(), String> {
    run_command("wpctl", args)
}

pub(crate) fn run_pw_metadata(args: &[String]) -> Result<(), String> {
    run_command("pw-metadata", args)
}

pub(crate) fn run_pw_link(args: &[String]) -> Result<(), String> {
    run_command("pw-link", args)
}

pub(crate) fn run_pactl(args: &[String]) -> Result<String, String> {
    let output = Command::new("pactl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run pactl: {e}"))?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "pactl exited with {}: {}",
            output.status,
            stderr.trim()
        ))
    }
}

pub(crate) fn unload_pactl_module(module_id: &str) -> Result<(), String> {
    if module_id.is_empty() {
        return Ok(());
    }
    let args = vec!["unload-module".to_string(), module_id.to_string()];
    run_pactl(&args).map(|_| ())
}

pub(crate) fn current_default_source_name() -> Result<Option<String>, String> {
    let args = vec!["info".to_string()];
    let raw = run_pactl(&args)?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Default Source:") {
            let name = rest.trim();
            if !name.is_empty() {
                return Ok(Some(name.to_string()));
            }
        }
    }
    Ok(None)
}

pub(crate) fn load_monitor_loopback_module(
    monitor_source_name: &str,
    output_device: &str,
) -> Result<String, String> {
    let args = vec![
        "load-module".to_string(),
        "module-loopback".to_string(),
        format!("source={monitor_source_name}"),
        format!("sink={output_device}"),
        "latency_msec=1".to_string(),
    ];
    run_pactl(&args).map(|stdout| stdout.trim().to_string())
}

pub(crate) fn rewire_virtual_mic_source(
    master_source: &str,
    virtual_source_name: &str,
) -> Result<String, String> {
    if let Some(module_id) = find_virtual_mic_module_id(virtual_source_name)? {
        unload_pactl_module(&module_id)?;
    }

    let args = vec![
        "load-module".to_string(),
        "module-remap-source".to_string(),
        format!("master={master_source}"),
        format!("source_name={virtual_source_name}"),
        format!("source_properties=device.description={virtual_source_name}"),
    ];
    run_pactl(&args).map(|stdout| stdout.trim().to_string())
}

pub(crate) fn ensure_virtual_devices(
    virtual_sinks: &[&str],
    virtual_sources: &[&str],
    legacy_sink_names: &[&str],
) -> Result<(), String> {
    unload_legacy_venturi_sinks(legacy_sink_names)?;

    let list_sinks_args = vec!["list".to_string(), "short".to_string(), "sinks".to_string()];
    let list_sources_args = vec![
        "list".to_string(),
        "short".to_string(),
        "sources".to_string(),
    ];
    let existing_sinks_raw = run_pactl(&list_sinks_args)?;
    let existing_sources_raw = run_pactl(&list_sources_args)?;

    let existing_sinks = parse_pactl_short_names(&existing_sinks_raw);
    let existing_sources = parse_pactl_short_names(&existing_sources_raw);

    for sink in virtual_sinks {
        if existing_sinks.contains(*sink) {
            continue;
        }
        let args = vec![
            "load-module".to_string(),
            "module-null-sink".to_string(),
            format!("sink_name={sink}"),
            format!("sink_properties=device.description={sink}"),
        ];
        run_pactl(&args)?;
    }

    for source in virtual_sources {
        if existing_sources.contains(*source) {
            continue;
        }

        let default_source = current_default_source_name()?.ok_or_else(|| {
            "no default source available to create Venturi virtual mic".to_string()
        })?;
        let args = vec![
            "load-module".to_string(),
            "module-remap-source".to_string(),
            format!("master={default_source}"),
            format!("source_name={source}"),
            format!("source_properties=device.description={source}"),
        ];
        run_pactl(&args)?;
    }

    Ok(())
}

fn parse_pactl_short_names(raw: &str) -> BTreeSet<String> {
    raw.lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let _index = cols.next()?;
            let name = cols.next()?;
            Some(name.to_string())
        })
        .collect()
}

fn unload_legacy_venturi_sinks(legacy_sink_names: &[&str]) -> Result<(), String> {
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let raw = run_pactl(&args)?;

    for line in raw.lines() {
        let mut cols = line.split_whitespace();
        let Some(module_id) = cols.next() else {
            continue;
        };
        let Some(module_name) = cols.next() else {
            continue;
        };
        if module_name != "module-null-sink" {
            continue;
        }

        if legacy_sink_names
            .iter()
            .any(|legacy| line.contains(&format!("sink_name={legacy}")))
        {
            let unload_args = vec!["unload-module".to_string(), module_id.to_string()];
            let _ = run_pactl(&unload_args)?;
        }
    }

    Ok(())
}

fn find_virtual_mic_module_id(virtual_source_name: &str) -> Result<Option<String>, String> {
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let raw = run_pactl(&args)?;

    for line in raw.lines() {
        if line.contains("module-remap-source")
            && line.contains(&format!("source_name={virtual_source_name}"))
        {
            let mut cols = line.split_whitespace();
            if let Some(id) = cols.next() {
                return Ok(Some(id.to_string()));
            }
        }
    }

    Ok(None)
}
