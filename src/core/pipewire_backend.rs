use std::collections::BTreeSet;
use std::process::Command;

const VENTURI_MAIN_OUTPUT: &str = "Venturi-Output";
const VENTURI_MAIN_MONITOR: &str = "Venturi-Output.monitor";
const VENTURI_VIRTUAL_MIC: &str = "Venturi-VirtualMic";
const MAIN_MIX_OUTPUT_DESCRIPTION: &str = "Venturi-MainMix-Output";
const MAIN_MIX_MONITOR_DESCRIPTION: &str = "Venturi-MainMix-Monitor";
const VIRTUAL_MIC_INPUT_DESCRIPTION: &str = "Venturi-Mic-Input";
const MAIN_MIX_ROUTE_APPLICATION_NAME: &str = "Venturi Main Mix Route";

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
    let args = build_monitor_loopback_load_args(monitor_source_name, output_device);
    run_pactl(&args).map(|stdout| stdout.trim().to_string())
}

pub(crate) fn reconcile_monitor_loopback_modules(
    monitor_source_name: &str,
    output_device: Option<&str>,
) -> Result<Option<String>, String> {
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let raw = run_pactl(&args)?;
    let plan = build_monitor_loopback_plan(&raw, monitor_source_name, output_device);

    for module_id in &plan.unload_ids {
        unload_pactl_module(module_id)?;
    }

    match plan.load_args {
        Some(load_args) => {
            let output_device = load_args
                .iter()
                .find_map(|arg| arg.strip_prefix("sink="))
                .ok_or_else(|| "missing sink for monitor loopback load plan".to_string())?;
            load_monitor_loopback_module(monitor_source_name, output_device).map(Some)
        }
        None => Ok(None),
    }
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
        format!(
            "source_properties={}",
            build_virtual_module_device_description_properties(source_description_for(
                virtual_source_name
            ))
        ),
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
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let modules_raw = run_pactl(&args)?;
    let unload_ids =
        collect_virtual_device_module_unload_ids(&modules_raw, virtual_sinks, virtual_sources);
    for module_id in &unload_ids {
        unload_pactl_module(module_id)?;
    }

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
            format!(
                "sink_properties={}",
                build_virtual_module_device_description_properties(sink_description_for(sink))
            ),
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
            format!(
                "source_properties={}",
                build_virtual_module_device_description_properties(source_description_for(source))
            ),
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

#[derive(Debug, PartialEq, Eq)]
struct MonitorLoopbackPlan {
    unload_ids: Vec<String>,
    load_args: Option<Vec<String>>,
}

fn build_monitor_loopback_plan(
    modules_raw: &str,
    monitor_source_name: &str,
    output_device: Option<&str>,
) -> MonitorLoopbackPlan {
    let unload_ids = modules_raw
        .lines()
        .filter_map(|line| {
            if !line.contains("module-loopback")
                || !line.contains(&format!("source={monitor_source_name}"))
            {
                return None;
            }
            line.split_whitespace().next().map(str::to_string)
        })
        .collect::<Vec<_>>();

    let load_args =
        output_device.map(|device| build_monitor_loopback_load_args(monitor_source_name, device));

    MonitorLoopbackPlan {
        unload_ids,
        load_args,
    }
}

fn build_monitor_loopback_load_args(monitor_source_name: &str, output_device: &str) -> Vec<String> {
    vec![
        "load-module".to_string(),
        "module-loopback".to_string(),
        format!("source={monitor_source_name}"),
        format!("sink={output_device}"),
        "latency_msec=1".to_string(),
        format!("sink_input_properties=application.name={MAIN_MIX_ROUTE_APPLICATION_NAME}"),
        format!("source_output_properties=application.name={MAIN_MIX_ROUTE_APPLICATION_NAME}"),
    ]
}

fn sink_description_for(sink_name: &str) -> &str {
    if sink_name == VENTURI_MAIN_OUTPUT {
        MAIN_MIX_OUTPUT_DESCRIPTION
    } else {
        sink_name
    }
}

fn source_description_for(source_name: &str) -> &str {
    if source_name == VENTURI_MAIN_MONITOR {
        MAIN_MIX_MONITOR_DESCRIPTION
    } else if source_name == VENTURI_VIRTUAL_MIC {
        VIRTUAL_MIC_INPUT_DESCRIPTION
    } else {
        source_name
    }
}

fn build_virtual_device_description_property(description: &str) -> String {
    format!("device.description={}", quote_proplist_value(description))
}

fn build_virtual_module_device_description_properties(description: &str) -> String {
    build_virtual_device_description_property(description)
}

fn collect_virtual_device_module_unload_ids(
    modules_raw: &str,
    virtual_sinks: &[&str],
    virtual_sources: &[&str],
) -> Vec<String> {
    modules_raw
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let module_id = cols.next()?;
            let module_name = cols.next()?;

            if module_name == "module-null-sink"
                && virtual_sinks
                    .iter()
                    .any(|sink| line.contains(&format!("sink_name={sink}")))
            {
                return Some(module_id.to_string());
            }

            if module_name == "module-remap-source"
                && virtual_sources
                    .iter()
                    .any(|source| line.contains(&format!("source_name={source}")))
            {
                return Some(module_id.to_string());
            }

            None
        })
        .collect()
}

fn quote_proplist_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
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

#[cfg(test)]
mod tests {
    use super::{
        build_monitor_loopback_plan, build_virtual_device_description_property,
        build_virtual_module_device_description_properties,
        collect_virtual_device_module_unload_ids, sink_description_for, source_description_for,
        MonitorLoopbackPlan,
    };

    #[test]
    fn plan_unloads_all_stale_venturi_monitor_loopbacks_and_loads_single_target() {
        let modules = r#"
536870916 module-loopback source=Venturi-Output.monitor sink=alsa_output.a latency_msec=1
536870917 module-loopback source=Venturi-Output.monitor sink=alsa_output.a latency_msec=1
536870918 module-loopback source=Venturi-Output.monitor sink=alsa_output.b latency_msec=1
536870999 module-loopback source=other.monitor sink=alsa_output.a latency_msec=1
"#;

        let plan = build_monitor_loopback_plan(
            modules,
            "Venturi-Output.monitor",
            Some("alsa_output.target"),
        );

        assert_eq!(
            plan,
            MonitorLoopbackPlan {
                unload_ids: vec![
                    "536870916".to_string(),
                    "536870917".to_string(),
                    "536870918".to_string()
                ],
                load_args: Some(vec![
                    "load-module".to_string(),
                    "module-loopback".to_string(),
                    "source=Venturi-Output.monitor".to_string(),
                    "sink=alsa_output.target".to_string(),
                    "latency_msec=1".to_string(),
                    "sink_input_properties=application.name=Venturi Main Mix Route".to_string(),
                    "source_output_properties=application.name=Venturi Main Mix Route".to_string(),
                ]),
            }
        );
    }

    #[test]
    fn plan_only_unloads_when_falling_back_to_default_output() {
        let modules = r#"
536870916 module-loopback source=Venturi-Output.monitor sink=alsa_output.a latency_msec=1
536870917 module-loopback source=Venturi-Output.monitor sink=alsa_output.b latency_msec=1
"#;

        let plan = build_monitor_loopback_plan(modules, "Venturi-Output.monitor", None);

        assert_eq!(
            plan,
            MonitorLoopbackPlan {
                unload_ids: vec!["536870916".to_string(), "536870917".to_string()],
                load_args: None,
            }
        );
    }

    #[test]
    fn uses_friendly_descriptions_for_main_virtual_devices() {
        assert_eq!(
            sink_description_for("Venturi-Output"),
            "Venturi-MainMix-Output"
        );
        assert_eq!(
            source_description_for("Venturi-Output.monitor"),
            "Venturi-MainMix-Monitor"
        );
        assert_eq!(
            source_description_for("Venturi-VirtualMic"),
            "Venturi-Mic-Input"
        );
    }

    #[test]
    fn builds_quoted_device_description_property() {
        assert_eq!(
            build_virtual_device_description_property("Venturi-MainMix-Output"),
            "device.description=\"Venturi-MainMix-Output\""
        );
    }

    #[test]
    fn builds_module_load_device_description_properties_only() {
        assert_eq!(
            build_virtual_module_device_description_properties("Venturi-Mic-Input"),
            "device.description=\"Venturi-Mic-Input\""
        );
    }

    #[test]
    fn collects_venturi_virtual_module_ids_for_recreation() {
        let modules = r#"
536870921 module-remap-source master=alsa_input.foo source_name=Venturi-VirtualMic source_properties=device.description="Venturi"
536870922 module-null-sink sink_name=Venturi-Output sink_properties=device.description=Venturi-Output
536870930 module-loopback source=Venturi-Output.monitor sink=alsa_output.bar latency_msec=1
536870999 module-remap-source master=alsa_input.foo source_name=OtherSource
"#;
        let virtual_sinks = ["Venturi-Output"];
        let virtual_sources = ["Venturi-VirtualMic"];

        let unload_ids = collect_virtual_device_module_unload_ids(
            modules,
            virtual_sinks.as_slice(),
            virtual_sources.as_slice(),
        );

        assert_eq!(
            unload_ids,
            vec!["536870921".to_string(), "536870922".to_string()]
        );
    }
}
