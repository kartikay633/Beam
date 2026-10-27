//! File-name hardening (SPEC.md section 6.4). Sender-supplied names are never
//! trusted: they are reduced to one safe path component before use.

use std::collections::HashSet;
use std::path::Path;

/// Longest sanitized name, in UTF-8 bytes. Leaves room under the usual
/// 255-byte component limit for " (123)" and the ".beam-part.json" suffix.
pub const MAX_FILE_NAME_BYTES: usize = 200;

/// Extensions longer than this are not preserved when truncating.
const MAX_KEPT_EXTENSION_BYTES: usize = 16;

/// Suffix of the temporary file a download is written to.
pub const PART_SUFFIX: &str = ".beam-part";

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4",
    "COM5", "COM6", "COM7", "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9",
];

/// Turns an untrusted name into a single, safe file-name component.
///
/// - Only the last path component is kept (`../../etc/passwd` → `passwd`).
/// - Characters Windows forbids (`<>:"/\|?*`) and control characters become `_`.
/// - Invisible bidirectional-override characters are removed, so
///   `report\u{202E}gpj.exe` can't pose as a `.jpg`.
/// - Leading/trailing whitespace and trailing dots are trimmed (Windows drops them).
/// - Windows device names (`CON`, `nul.txt`, `LPT1.tar.gz`, …) get a `_` prefix.
/// - Names are truncated to [`MAX_FILE_NAME_BYTES`], keeping a short extension.
/// - An empty result becomes `file`.
pub fn sanitize_file_name(raw: &str) -> String {
    let last_component = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last_component
        .chars()
        .filter(|&c| !is_invisible_formatting(c))
        .map(|c| if is_forbidden(c) { '_' } else { c })
        .collect();

    let mut name = trim_name(&cleaned).to_string();
    if name.is_empty() {
        return "file".to_string();
    }
    if is_windows_reserved(&name) {
        name.insert(0, '_');
    }
    truncate_name(&name, MAX_FILE_NAME_BYTES)
}

/// Picks a name in `dir` that is not already used by a file, a partial
/// download, or another name in `taken`: `photo.jpg`, `photo (1).jpg`, …
/// `taken` holds lower-cased names because Windows file names are
/// case-insensitive.
pub fn unique_file_name(dir: &Path, name: &str, taken: &HashSet<String>) -> String {
    let (stem, ext) = split_extension(name);
    let mut candidate = name.to_string();
    let mut counter = 1u32;
    while is_name_used(dir, &candidate, taken) {
        candidate = format!("{stem} ({counter}){ext}");
        counter += 1;
    }
    candidate
}

fn is_name_used(dir: &Path, name: &str, taken: &HashSet<String>) -> bool {
    taken.contains(&name.to_lowercase())
        || dir.join(name).exists()
        || dir.join(format!("{name}{PART_SUFFIX}")).exists()
}

fn is_forbidden(c: char) -> bool {
    c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

fn is_invisible_formatting(c: char) -> bool {
    matches!(
        c,
        '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
    )
}

fn trim_name(name: &str) -> &str {
    name.trim()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
}

fn is_windows_reserved(name: &str) -> bool {
    // Windows treats "CON", "con.txt" and "CON .tar.gz" all as the device.
    let base = name.split('.').next().unwrap_or_default().trim_end();
    WINDOWS_RESERVED
        .iter()
        .any(|reserved| base.eq_ignore_ascii_case(reserved))
}

/// Splits `photo.tar.gz` into (`photo.tar`, `.gz`). Hidden-file names like
/// `.bashrc` have no extension.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

fn truncate_name(name: &str, max_bytes: usize) -> String {
    if name.len() <= max_bytes {
        return name.to_string();
    }
    let (stem, ext) = match split_extension(name) {
        (stem, ext) if ext.len() <= MAX_KEPT_EXTENSION_BYTES => (stem, ext),
        _ => (name, ""),
    };
    let stem = truncate_at_char_boundary(stem, max_bytes - ext.len());
    let stem = trim_name(stem);
    if stem.is_empty() {
        return truncate_at_char_boundary(name, max_bytes).to_string();
    }
    format!("{stem}{ext}")
}

fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nasty_inputs_are_sanitized() {
        let cases: &[(&str, &str)] = &[
            // Path traversal and absolute paths
            ("../../etc/passwd", "passwd"),
            ("..\\..\\Windows\\System32\\cmd.exe", "cmd.exe"),
            ("/etc/shadow", "shadow"),
            ("C:\\Users\\victim\\evil.txt", "evil.txt"),
            ("\\\\server\\share\\x.txt", "x.txt"),
            ("a\\b", "b"),
            ("a/b/", "file"),
            ("..", "file"),
            (".", "file"),
            ("...", "file"),
            ("../", "file"),
            // Drive letters and alternate data streams
            ("C:evil.txt", "C_evil.txt"),
            ("file.txt:hidden_stream", "file.txt_hidden_stream"),
            // Windows reserved device names
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("CON.txt", "_CON.txt"),
            ("Lpt1.tar.gz", "_Lpt1.tar.gz"),
            ("nul", "_nul"),
            ("COM9.log", "_COM9.log"),
            ("CON .txt", "_CON .txt"),
            ("CONSOLE.txt", "CONSOLE.txt"),
            ("icon.png", "icon.png"),
            // Forbidden and control characters
            ("bad\0name\n.txt", "bad_name_.txt"),
            ("tab\there.txt", "tab_here.txt"),
            ("bell\u{7}.txt", "bell_.txt"),
            ("what?.txt", "what_.txt"),
            ("a<b>c|d*e\"f.txt", "a_b_c_d_e_f.txt"),
            ("c1\u{85}control.txt", "c1_control.txt"),
            // Invisible bidi tricks
            ("report\u{202E}gpj.exe", "reportgpj.exe"),
            ("\u{FEFF}bom.txt", "bom.txt"),
            // Trailing dots and spaces, which Windows silently strips
            ("photo.jpg. . ", "photo.jpg"),
            ("  spaced.txt  ", "spaced.txt"),
            ("", "file"),
            ("   ", "file"),
            // Things that must pass through unchanged
            (".bashrc", ".bashrc"),
            ("holiday photo (1).jpg", "holiday photo (1).jpg"),
            ("日本語のファイル.txt", "日本語のファイル.txt"),
            ("emoji 🎉.png", "emoji 🎉.png"),
        ];
        for (input, expected) in cases {
            assert_eq!(&sanitize_file_name(input), expected, "input: {input:?}");
        }
    }

    #[test]
    fn long_names_are_truncated_keeping_the_extension() {
        let name = sanitize_file_name(&format!("{}.jpg", "a".repeat(300)));
        assert_eq!(name.len(), MAX_FILE_NAME_BYTES);
        assert!(name.ends_with("a.jpg"));
    }

    #[test]
    fn long_multibyte_names_are_cut_on_a_char_boundary() {
        let name = sanitize_file_name(&"é".repeat(300));
        assert!(name.len() <= MAX_FILE_NAME_BYTES);
        assert!(name.chars().all(|c| c == 'é'));
    }

    #[test]
    fn very_long_extensions_are_not_preserved() {
        let name = sanitize_file_name(&format!("x.{}", "e".repeat(300)));
        assert_eq!(name.len(), MAX_FILE_NAME_BYTES);
    }

    #[test]
    fn sanitized_names_are_always_single_safe_components() {
        let inputs = [
            "../a",
            "a/../../b",
            "C:/x",
            "\\\\?\\C:\\x",
            "...\\...",
            "a/./b",
        ];
        for input in inputs {
            let name = sanitize_file_name(input);
            assert!(!name.is_empty());
            assert!(!name.contains(['/', '\\', ':']), "{input:?} → {name:?}");
            assert_ne!(name, "..");
            assert_ne!(name, ".");
        }
    }

    #[test]
    fn unique_names_skip_existing_files_parts_and_reserved_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut taken = HashSet::new();
        assert_eq!(
            unique_file_name(dir.path(), "photo.jpg", &taken),
            "photo.jpg"
        );

        std::fs::write(dir.path().join("photo.jpg"), b"x").unwrap();
        assert_eq!(
            unique_file_name(dir.path(), "photo.jpg", &taken),
            "photo (1).jpg"
        );

        std::fs::write(dir.path().join("photo (1).jpg.beam-part"), b"x").unwrap();
        taken.insert("photo (2).jpg".to_string());
        assert_eq!(
            unique_file_name(dir.path(), "photo.jpg", &taken),
            "photo (3).jpg"
        );

        assert_eq!(unique_file_name(dir.path(), ".bashrc", &taken), ".bashrc");
        std::fs::write(dir.path().join(".bashrc"), b"x").unwrap();
        assert_eq!(
            unique_file_name(dir.path(), ".bashrc", &taken),
            ".bashrc (1)"
        );
    }
}
