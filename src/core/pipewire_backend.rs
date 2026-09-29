use std::collections::BTreeSet;
use std::io::Read;
use std::process::Command;
use std::process::{Child, ChildStdout, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const VENTURI_MAIN_OUTPUT: &str = "Venturi-Output";
const VENTURI_MAIN_MONITOR: &str = "Venturi-Output.monitor";
const VENTURI_VIRTUAL_MIC: &str = "Venturi-VirtualMic";
const MAIN_MIX_OUTPUT_DESCRIPTION: &str = "Venturi-MainMix-Output";
const MAIN_MIX_MONITOR_DESCRIPTION: &str = "Venturi-MainMix-Monitor";
const VIRTUAL_MIC_INPUT_DESCRIPTION: &str = "Venturi-Mic-Input";
const MAIN_MIX_ROUTE_APPLICATION_NAME: &str = "Venturi Main Mix Route";

/// Upper bound for any one-shot `pactl`/`wpctl`/`pw-*` invocation.
///
/// These tools block until the PipeWire graph acknowledges the request. When the
/// graph is wedged (seen after suspend/resume: links stuck in `init`, nodes never
/// becoming runnable) a call like `pactl load-module module-loopback` never
/// returns, which would freeze the core loop and with it every command from the
/// GUI, hotkeys and tray. A healthy call finishes in tens of milliseconds, so
/// hitting this limit is itself a signal that PipeWire is unhealthy.
pub(crate) const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(10);

const SUBPROCESS_POLL_INTERVAL: Duration = Duration::from_millis(2);

struct CapturedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Run `program` to completion, killing it if it exceeds `timeout`.
///
/// stdout and stderr are drained on helper threads so a chatty child can't
/// block on a full pipe while we wait for it.
fn run_with_timeout(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<CapturedOutput, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {program}: {e}"))?;

    let stdout_reader = spawn_drain(child.stdout.take());
    let stderr_reader = spawn_drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed waiting for {program}: {e}"));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{program} {} did not finish within {}s (PipeWire unresponsive?)",
                args.join(" "),
                timeout.as_secs()
            ));
        }
        std::thread::sleep(SUBPROCESS_POLL_INTERVAL);
    };

    Ok(CapturedOutput {
        status,
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    })
}

fn spawn_drain<R: Read + Send + 'static>(reader: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut reader) = reader {
            let _ = reader.read_to_end(&mut buf);
        }
        buf
    })
}

fn run_captured(program: &str, args: &[String]) -> Result<CapturedOutput, String> {
    run_with_timeout(program, args, SUBPROCESS_TIMEOUT)
}

fn describe_failure(program: &str, output: &CapturedOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{program} exited with {}: {}", output.status, stderr.trim())
}

pub(crate) fn run_command(program: &str, args: &[String]) -> Result<(), String> {
    let output = run_captured(program, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(describe_failure(program, &output))
    }
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
    let output = run_captured("wpctl", &args)?;

    if !output.status.success() {
        return Err(describe_failure("wpctl", &output));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_wpctl_volume_output(&stdout)
        .ok_or_else(|| format!("unable to parse wpctl get-volume output: {stdout:?}"))
}

pub(crate) fn run_pw_metadata(args: &[String]) -> Result<(), String> {
    run_command("pw-metadata", args)
}

pub(crate) fn run_pactl(args: &[String]) -> Result<String, String> {
    let output = run_captured("pactl", args)?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        Err(describe_failure("pactl", &output))
    }
}

/// Marker property set on all Venturi-spawned `pw-play` processes so the
/// discovery layer can recognise (and skip) them during auto-routing.
pub(crate) const SOUNDBOARD_APP_NAME: &str = "Venturi-Soundboard";

fn build_pw_play_args(target: &str, file: &str) -> Vec<String> {
    vec![
        "--target".to_string(),
        target.to_string(),
        "--properties".to_string(),
        format!("application.name={SOUNDBOARD_APP_NAME}"),
        file.to_string(),
    ]
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

/// Point the main-mix `module-loopback` at `output_device`.
///
/// An existing module whose `sink=` argument already matches is kept (no
/// audio pop) unless `force_reload` is set. The module arguments only say what
/// the loopback was *asked* to target; after suspend/resume or a device
/// unplug/replug its streams can be dead while the module still lists fine, so
/// restore paths must recreate it regardless.
pub(crate) fn reconcile_monitor_loopback_modules(
    monitor_source_name: &str,
    output_device: Option<&str>,
    force_reload: bool,
) -> Result<Option<String>, String> {
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let raw = run_pactl(&args)?;
    let plan = build_monitor_loopback_plan(&raw, monitor_source_name, output_device, force_reload);

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

/// Point the `module-remap-source` behind the virtual mic at `master_source`.
///
/// When a module already exists for the same master it is kept (no audio pop)
/// unless `force_reload` is set, which is used after the master device was
/// unplugged and reappeared: the remap module then silently follows
/// WirePlumber's default source, so it must be recreated to re-attach.
///
/// Reloading the module recreates the `input.<virtual_source_name>` node, which
/// drops the soundboard pw-link, so the link is re-established here.
pub(crate) fn rewire_virtual_mic_source(
    master_source: &str,
    virtual_source_name: &str,
    force_reload: bool,
) -> Result<String, String> {
    if let Some((module_id, existing_master)) = find_virtual_mic_module(virtual_source_name)? {
        if !force_reload && existing_master.as_deref() == Some(master_source) {
            return Ok(module_id);
        }
        unload_pactl_module(&module_id)?;
    }

    let module_id = load_virtual_mic_module(master_source, virtual_source_name)?;
    link_soundboard_to_virtual_mic(VENTURI_SOUND_SINK, virtual_source_name);
    Ok(module_id)
}

fn load_virtual_mic_module(
    master_source: &str,
    virtual_source_name: &str,
) -> Result<String, String> {
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

/// Create any missing Venturi null sinks / virtual mic and wire every category
/// sink's monitor into `Venturi-Output`.
///
/// Null sinks are never recreated when present (apps are connected to them).
/// The internal channel→main loopbacks are kept unless `force_reload_loopbacks`
/// is set; the resume path sets it because after suspend those `module-loopback`
/// streams were observed still listed and "running" yet no longer carrying
/// audio into the main mix.
pub(crate) fn ensure_virtual_devices(
    virtual_sinks: &[&str],
    virtual_sources: &[&str],
    legacy_sink_names: &[&str],
    force_reload_loopbacks: bool,
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
        reconcile_monitor_loopback_modules(
            &monitor_source,
            Some(VENTURI_MAIN_OUTPUT),
            force_reload_loopbacks,
        )
        .map_err(|err| {
                format!(
                    "failed to route category mix monitor {monitor_source} into {VENTURI_MAIN_OUTPUT}: {err}"
                )
            })?;
    }

    // Route the soundboard sink's monitor into the virtual mic input via pw-link.
    // This uses port-level linking because the virtual mic is a source, not a sink,
    // so module-loopback can't target it.
    if let Some(sound_sink) = virtual_sinks
        .iter()
        .find(|s| s.eq_ignore_ascii_case(VENTURI_SOUND_SINK))
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
    let modules_raw = run_pactl(&[
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ])?;
    for line in modules_raw.lines() {
        if line.contains("module-null-sink")
            && line.contains(&format!("sink_name={sink_name}"))
            && let Some(module_id) = line.split_whitespace().next()
        {
            unload_pactl_module(module_id)?;
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
    let output = run_captured("pw-link", &["-o".to_string()])?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        Err(describe_failure("pw-link -o", &output))
    }
}

/// Create a pw-link. An already-existing link ("File exists") is success.
fn run_pw_link(args: &[String]) -> Result<(), String> {
    let output = run_captured("pw-link", args)?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if pw_link_result_is_success(output.status.success(), &stderr) {
        Ok(())
    } else {
        Err(describe_failure("pw-link", &output))
    }
}

fn pw_link_result_is_success(status_ok: bool, stderr: &str) -> bool {
    status_ok || stderr.to_ascii_lowercase().contains("exists")
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
    force_reload: bool,
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
    if !force_reload
        && let Some(desired) = output_device
        && matching_modules.len() == 1
        && matching_modules[0].1.as_deref() == Some(desired)
    {
        return MonitorLoopbackPlan {
            unload_ids: vec![],
            load_args: None,
        };
    }

    let unload_ids = matching_modules.into_iter().map(|(id, _)| id).collect();

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
        MonitorLoopbackPlan, SOUNDBOARD_APP_NAME, build_monitor_loopback_plan, build_pw_play_args,
        build_virtual_device_description_property,
        build_virtual_module_device_description_properties, category_mix_monitor_sources,
        collect_virtual_device_module_unload_ids, compute_stereo_peak_from_s16le,
        find_virtual_mic_module_in_modules_raw, parse_wpctl_volume_output,
        pw_link_result_is_success, run_with_timeout, sink_description_for, source_description_for,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn hung_subprocess_is_killed_and_reported_after_timeout() {
        let started = Instant::now();
        let result = run_with_timeout("sleep", &["30".to_string()], Duration::from_millis(200));

        let err = result.err().expect("timed-out command must fail");
        assert!(err.contains("did not finish"), "unexpected error: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout must not wait for the child to finish on its own"
        );
    }

    #[test]
    fn fast_subprocess_output_and_status_are_captured() {
        let script = "printf out; printf err >&2; exit 3".to_string();
        let output = run_with_timeout("sh", &["-c".to_string(), script], Duration::from_secs(5))
            .expect("sh should run");

        assert_eq!(output.status.code(), Some(3));
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
    }

    #[test]
    fn large_subprocess_output_does_not_deadlock() {
        // > 64 KiB (the pipe buffer): would hang if we waited before draining.
        let script = "head -c 300000 /dev/zero".to_string();
        let output = run_with_timeout("sh", &["-c".to_string(), script], Duration::from_secs(5))
            .expect("sh should run");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 300_000);
    }

    #[test]
    fn pw_link_treats_existing_link_as_success() {
        assert!(pw_link_result_is_success(true, ""));
        assert!(pw_link_result_is_success(
            false,
            "failed to link ports: File exists\n"
        ));
        assert!(!pw_link_result_is_success(
            false,
            "failed to link ports: No such file or directory\n"
        ));
    }

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
            false,
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

        let plan = build_monitor_loopback_plan(modules, "Venturi-Output.monitor", None, false);

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
            false,
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
    fn plan_force_reload_recreates_loopback_even_when_target_already_matches() {
        // After suspend/resume the module still lists with the right `sink=`
        // but its streams may be dead; a forced restore must unload + reload.
        let modules = r#"
536870916 module-loopback source=Venturi-Output.monitor sink=alsa_output.target latency_msec=1
"#;

        let plan = build_monitor_loopback_plan(
            modules,
            "Venturi-Output.monitor",
            Some("alsa_output.target"),
            true,
        );

        assert_eq!(plan.unload_ids, vec!["536870916".to_string()]);
        assert!(
            plan.load_args
                .as_deref()
                .is_some_and(|args| args.contains(&"sink=alsa_output.target".to_string()))
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
                "--properties".to_string(),
                format!("application.name={SOUNDBOARD_APP_NAME}"),
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
