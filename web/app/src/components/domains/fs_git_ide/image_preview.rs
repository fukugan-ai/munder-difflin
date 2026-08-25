use dioxus::prelude::*;
use md_web_contracts::domains::fs_git_ide::BinaryFile;

#[component]
pub(super) fn ImagePreview(file: BinaryFile) -> Element {
    let supported = matches!(
        file.mime.as_str(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/bmp"
            | "image/x-icon"
            | "image/avif"
            | "image/svg+xml"
    );
    if !supported {
        return rsx! { p { class: "md-ide-note", "このバイナリ形式はプレビューできません" } };
    }
    let source = format!("data:{};base64,{}", file.mime, encode_base64(&file.bytes));
    rsx! {
        figure { class: "md-image-preview",
            img { src: source, alt: "{file.rel_path} のプレビュー" }
            figcaption { "{file.rel_path} · {file.size} bytes" }
        }
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(TABLE[usize::from(first >> 2)]));
        encoded.push(char::from(
            TABLE[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        encoded.push(if chunk.len() > 1 {
            char::from(TABLE[usize::from(((second & 0x0f) << 2) | (third >> 6))])
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            char::from(TABLE[usize::from(third & 0x3f)])
        } else {
            '='
        });
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::encode_base64;

    #[test]
    fn base64_handles_complete_and_padded_groups() {
        assert_eq!(encode_base64(b"Man"), "TWFu");
        assert_eq!(encode_base64(b"Ma"), "TWE=");
        assert_eq!(encode_base64(b"M"), "TQ==");
    }
}
