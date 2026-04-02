use std::collections::HashSet;

pub fn collision_safe_name(existing: &HashSet<String>, file_name: &str) -> String {
    if !existing.contains(file_name) {
        return file_name.to_string();
    }

    let (base, ext) = file_name
        .rsplit_once('.')
        .map(|(b, e)| (b.to_string(), format!(".{e}")))
        .unwrap_or_else(|| (file_name.to_string(), String::new()));

    let mut i = 1;
    loop {
        let candidate = format!("{base}-{i}{ext}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        i += 1;
    }
}
