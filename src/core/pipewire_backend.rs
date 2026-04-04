use std::collections::BTreeSet;
use std::io::Read;
use std::process::Command;
use std::process::{Child, ChildStdout, Stdio};

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

fn parse_wpctl_volume_output(output: &str) -> Option<f32> {
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("Volume:") else {
            continue;
        };

        for token in rest.split_whitespace() {
            if let Ok(value) = token.parse::<f32>() {
                return Some(value);
            }
        }
    }
    None
}

pub(crate) fn read_wpctl_volume(target: &str) -> Result<f32, String> {
    let args = vec!["get-volume".to_string(), target.to_string()];
    let output = Command::new("wpctl")
        .args(&args)
        .output()
        .map_err(|e| format!("failed to run wpctl: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "wpctl exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_wpctl_volume_output(&stdout)
        .ok_or_else(|| format!("unable to parse wpctl get-volume output: {stdout:?}"))
}

pub(crate) fn run_pw_metadata(args: &[String]) -> Result<(), String> {
    run_command("pw-metadata", args)
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

fn build_pw_play_args(target: &str, file: &str) -> Vec<String> {
    vec!["--target".to_string(), target.to_string(), file.to_string()]
}

pub(crate) struct PwPlayProcess {
    child: Child,
}

impl PwPlayProcess {
    pub(crate) fn spawn(target: &str, file: &str) -> Result<Self, String> {
        let args = build_pw_play_args(target, file);
        let child = Command::new("pw-play")
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn pw-play: {e}"))?;

        Ok(Self { child })
    }

    pub(crate) fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub(crate) fn is_finished(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_status)) => true,
            Ok(None) => false,
            Err(_) => true,
        }
    }
}

impl Drop for PwPlayProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct PwTargetSampler {
    child: Child,
    stdout: ChildStdout,
}

impl PwTargetSampler {
    pub(crate) fn spawn(target: &str) -> Result<Self, String> {
        let args = vec![
            "--target".to_string(),
            target.to_string(),
            "--rate".to_string(),
            "48000".to_string(),
            "--channels".to_string(),
            "2".to_string(),
            "--format".to_string(),
            "s16".to_string(),
            "--raw".to_string(),
            "-".to_string(),
        ];

        let mut child = Command::new("pw-record")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn pw-record sampler: {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture pw-record stdout".to_string())?;

        Ok(Self { child, stdout })
    }

    pub(crate) fn sample_levels(&mut self, sample_count: u32) -> Result<(f32, f32), String> {
        let byte_len = sample_count.saturating_mul(4) as usize;
        let mut raw = vec![0_u8; byte_len];
        self.stdout
            .read_exact(&mut raw)
            .map_err(|e| format!("failed reading pw-record sampler output: {e}"))?;
        Ok(compute_stereo_peak_from_s16le(raw.as_slice()))
    }
}

impl Drop for PwTargetSampler {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

pub(crate) fn current_default_sink_name() -> Result<Option<String>, String> {
    let args = vec!["info".to_string()];
    let raw = run_pactl(&args)?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Default Sink:") {
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
    if let Some((module_id, existing_master)) = find_virtual_mic_module(virtual_source_name)? {
        if existing_master.as_deref() == Some(master_source) {
            return Ok(module_id);
        }
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
            // Recreate Venturi-Sound as mono if it already exists as stereo.
            if sink.eq_ignore_ascii_case(VENTURI_SOUND_SINK) {
                recreate_sound_sink_as_mono_if_needed(sink)?;
            }
            continue;
        }
        let mut args = vec![
            "load-module".to_string(),
            "module-null-sink".to_string(),
            format!("sink_name={sink}"),
        ];
        if sink.eq_ignore_ascii_case(VENTURI_SOUND_SINK) {
            args.push("channels=1".to_string());
            args.push("channel_map=mono".to_string());
        }
        args.push(format!(
            "sink_properties={}",
            build_virtual_module_device_description_properties(sink_description_for(sink))
        ));
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

    for monitor_source in category_mix_monitor_sources(virtual_sinks) {
        reconcile_monitor_loopback_modules(&monitor_source, Some(VENTURI_MAIN_OUTPUT)).map_err(
            |err| {
                format!(
                    "failed to route category mix monitor {monitor_source} into {VENTURI_MAIN_OUTPUT}: {err}"
                )
            },
        )?;
    }

    // Route the soundboard sink's monitor into the virtual mic input via pw-link.
    // This uses port-level linking because the virtual mic is a source, not a sink,
    // so module-loopback can't target it.
    if let Some(sound_sink) = virtual_sinks.iter().find(|s| s.eq_ignore_ascii_case(VENTURI_SOUND_SINK))
        && let Some(virtual_mic) = virtual_sources.first()
    {
        link_soundboard_to_virtual_mic(sound_sink, virtual_mic);
    }

    Ok(())
}

const VENTURI_SOUND_SINK: &str = "Venturi-Sound";

fn link_soundboard_to_virtual_mic(sound_sink: &str, virtual_mic: &str) {
    // Venturi-Sound is mono, VirtualMic input is mono — link monitor_MONO to input_MONO.
    // Retry a few times since the sink ports may not be ready immediately after creation.
    let args = vec![
        format!("{sound_sink}:monitor_MONO"),
        format!("input.{virtual_mic}:input_MONO"),
    ];
    for attempt in 0..5 {
        if run_pw_link(&args).is_ok() {
            return;
        }
        if attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

fn recreate_sound_sink_as_mono_if_needed(sink_name: &str) -> Result<(), String> {
    // Check if the existing sink has a monitor_MONO port (mono) or monitor_FL (stereo).
    let output = run_pw_link_list_outputs()?;
    let has_mono = output
        .lines()
        .any(|line| line.trim() == format!("{sink_name}:monitor_MONO"));
    if has_mono {
        return Ok(());
    }

    // Existing sink is stereo — unload and recreate as mono.
    let modules_raw = run_pactl(&vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ])?;
    for line in modules_raw.lines() {
        if line.contains("module-null-sink") && line.contains(&format!("sink_name={sink_name}")) {
            if let Some(module_id) = line.split_whitespace().next() {
                unload_pactl_module(module_id)?;
            }
        }
    }
    let args = vec![
        "load-module".to_string(),
        "module-null-sink".to_string(),
        format!("sink_name={sink_name}"),
        "channels=1".to_string(),
        "channel_map=mono".to_string(),
        format!(
            "sink_properties={}",
            build_virtual_module_device_description_properties(sink_description_for(sink_name))
        ),
    ];
    run_pactl(&args)?;
    Ok(())
}

fn run_pw_link_list_outputs() -> Result<String, String> {
    let output = std::process::Command::new("pw-link")
        .arg("-o")
        .output()
        .map_err(|e| format!("failed to run pw-link -o: {e}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        Err("pw-link -o failed".to_string())
    }
}

fn run_pw_link(args: &[String]) -> Result<(), String> {
    run_command("pw-link", args)
}

fn category_mix_monitor_sources(virtual_sinks: &[&str]) -> Vec<String> {
    virtual_sinks
        .iter()
        .filter(|sink_name| {
            !sink_name.eq_ignore_ascii_case(VENTURI_MAIN_OUTPUT)
                && !sink_name.eq_ignore_ascii_case(VENTURI_SOUND_SINK)
        })
        .map(|sink_name| format!("{sink_name}.monitor"))
        .collect()
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
    let mut matching_modules: Vec<(String, Option<String>)> = Vec::new();

    for line in modules_raw.lines() {
        if !line.contains("module-loopback")
            || !line.contains(&format!("source={monitor_source_name}"))
        {
            continue;
        }
        let Some(module_id) = line.split_whitespace().next() else {
            continue;
        };
        let sink = line
            .split_whitespace()
            .find_map(|token| token.strip_prefix("sink="))
            .map(str::to_string);
        matching_modules.push((module_id.to_string(), sink));
    }

    // If there's exactly one existing loopback already pointing at the desired target,
    // keep it to avoid an audio pop from unnecessary unload/reload.
    if let Some(desired) = output_device
        && matching_modules.len() == 1
        && matching_modules[0].1.as_deref() == Some(desired)
    {
        return MonitorLoopbackPlan {
            unload_ids: vec![],
            load_args: None,
        };
    }

    let unload_ids = matching_modules
        .into_iter()
        .map(|(id, _)| id)
        .collect();

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
    let mut seen_virtual_sinks = BTreeSet::new();
    let mut seen_virtual_sources = BTreeSet::new();

    modules_raw
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let module_id = cols.next()?;
            let module_name = cols.next()?;

            if module_name == "module-null-sink" {
                let sink_name = line
                    .split_whitespace()
                    .find_map(|token| token.strip_prefix("sink_name="))?;
                if virtual_sinks.contains(&sink_name) {
                    if seen_virtual_sinks.insert(sink_name.to_string()) {
                        return None;
                    }
                    return Some(module_id.to_string());
                }
            }

            if module_name == "module-remap-source" {
                let source_name = line
                    .split_whitespace()
                    .find_map(|token| token.strip_prefix("source_name="))?;
                if virtual_sources.contains(&source_name) {
                    if seen_virtual_sources.insert(source_name.to_string()) {
                        return None;
                    }
                    return Some(module_id.to_string());
                }
            }

            None
        })
        .collect()
}

fn quote_proplist_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn compute_stereo_peak_from_s16le(raw: &[u8]) -> (f32, f32) {
    let mut left_peak = 0.0f32;
    let mut right_peak = 0.0f32;

    for frame in raw.chunks_exact(4) {
        let left = i16::from_le_bytes([frame[0], frame[1]]);
        let right = i16::from_le_bytes([frame[2], frame[3]]);
        let left_norm = (left as f32).abs() / i16::MAX as f32;
        let right_norm = (right as f32).abs() / i16::MAX as f32;
        left_peak = left_peak.max(left_norm);
        right_peak = right_peak.max(right_norm);
    }

    (left_peak.clamp(0.0, 1.0), right_peak.clamp(0.0, 1.0))
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

fn find_virtual_mic_module(
    virtual_source_name: &str,
) -> Result<Option<(String, Option<String>)>, String> {
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let raw = run_pactl(&args)?;

    Ok(find_virtual_mic_module_in_modules_raw(
        &raw,
        virtual_source_name,
    ))
}

fn find_virtual_mic_module_in_modules_raw(
    modules_raw: &str,
    virtual_source_name: &str,
) -> Option<(String, Option<String>)> {
    for line in modules_raw.lines() {
        let mut cols = line.split_whitespace();
        let Some(module_id) = cols.next() else {
            continue;
        };
        let Some(module_name) = cols.next() else {
            continue;
        };

        if module_name != "module-remap-source" {
            continue;
        }

        let source_name = line
            .split_whitespace()
            .find_map(|token| token.strip_prefix("source_name="));
        if source_name != Some(virtual_source_name) {
            continue;
        }

        let master_source = line
            .split_whitespace()
            .find_map(|token| token.strip_prefix("master="))
            .map(str::to_string);

        return Some((module_id.to_string(), master_source));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        MonitorLoopbackPlan, build_monitor_loopback_plan, build_pw_play_args,
        build_virtual_device_description_property,
        build_virtual_module_device_description_properties, category_mix_monitor_sources,
        collect_virtual_device_module_unload_ids, compute_stereo_peak_from_s16le,
        find_virtual_mic_module_in_modules_raw, parse_wpctl_volume_output, sink_description_for,
        source_description_for,
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
    fn plan_keeps_single_correct_loopback_to_avoid_pop() {
        let modules = r#"
536870916 module-loopback source=Venturi-Game.monitor sink=Venturi-Output latency_msec=1
"#;

        let plan = build_monitor_loopback_plan(
            modules,
            "Venturi-Game.monitor",
            Some("Venturi-Output"),
        );

        assert_eq!(
            plan,
            MonitorLoopbackPlan {
                unload_ids: vec![],
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
    fn does_not_collect_single_virtual_device_modules_for_unload() {
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

        assert!(unload_ids.is_empty());
    }

    #[test]
    fn collects_duplicate_virtual_device_module_ids_for_unload() {
        let modules = r#"
536870920 module-remap-source master=alsa_input.a source_name=Venturi-VirtualMic
536870921 module-remap-source master=alsa_input.b source_name=Venturi-VirtualMic
536870922 module-null-sink sink_name=Venturi-Output
536870923 module-null-sink sink_name=Venturi-Output
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
            vec!["536870921".to_string(), "536870923".to_string()]
        );
    }

    #[test]
    fn finds_virtual_mic_module_with_current_master_source() {
        let modules = r#"
536870920 module-remap-source master=alsa_input.a source_name=Venturi-VirtualMic
536870921 module-remap-source master=alsa_input.b source_name=OtherSource
"#;

        let info = find_virtual_mic_module_in_modules_raw(modules, "Venturi-VirtualMic");

        assert_eq!(
            info,
            Some(("536870920".to_string(), Some("alsa_input.a".to_string())))
        );
    }

    #[test]
    fn does_not_find_virtual_mic_module_for_other_source_names() {
        let modules = r#"
536870920 module-remap-source master=alsa_input.a source_name=OtherSource
"#;

        let info = find_virtual_mic_module_in_modules_raw(modules, "Venturi-VirtualMic");

        assert_eq!(info, None);
    }

    #[test]
    fn computes_stereo_peak_levels_from_s16le_pcm() {
        let raw: [u8; 12] = [
            0x00, 0x00, 0x00, 0x00, // frame 1: silence
            0xff, 0x3f, 0x00, 0x20, // frame 2: left ~0.5, right ~0.25
            0x00, 0x20, 0xff, 0x5f, // frame 3: left ~0.25, right ~0.75
        ];
        let (left, right) = compute_stereo_peak_from_s16le(raw.as_slice());
        assert!((left - 0.5).abs() < 0.02);
        assert!((right - 0.75).abs() < 0.02);
    }

    #[test]
    fn parses_wpctl_volume_output_basic_value() {
        let output = "Volume: 0.25\n";
        assert_eq!(parse_wpctl_volume_output(output), Some(0.25));
    }

    #[test]
    fn parses_wpctl_volume_output_with_muted_suffix() {
        let output = "Volume: 0.88 [MUTED]\n";
        assert_eq!(parse_wpctl_volume_output(output), Some(0.88));
    }

    #[test]
    fn builds_pw_play_args_with_target_and_file() {
        let args = build_pw_play_args("input.Venturi-VirtualMic", "/tmp/airhorn.wav");
        assert_eq!(
            args,
            vec![
                "--target".to_string(),
                "input.Venturi-VirtualMic".to_string(),
                "/tmp/airhorn.wav".to_string()
            ]
        );
    }

    #[test]
    fn collects_category_mix_monitor_sources_excluding_output_and_sound() {
        let virtual_sinks = [
            "Venturi-Output",
            "Venturi-Game",
            "Venturi-Media",
            "Venturi-Sound",
        ];

        let monitor_sources = category_mix_monitor_sources(virtual_sinks.as_slice());

        assert_eq!(
            monitor_sources,
            vec![
                "Venturi-Game.monitor".to_string(),
                "Venturi-Media.monitor".to_string()
            ]
        );
    }
}
