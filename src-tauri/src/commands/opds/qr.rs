//! QR codes of catalog URLs, so a phone or tablet can pick up the LAN
//! address from Settings › OPDS instead of having it typed.

use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};

use crate::error::{AppError, AppResult};

/// Longest text accepted (a catalog URL is about 30 characters; this leaves
/// room for anything sensible and keeps the code scannable).
pub(super) const MAX_QR_TEXT: usize = 512;

/// `text` as a QR code in SVG: black modules on white with the standard
/// quiet zone (scanners need both, whatever the app's theme), at least
/// 256 px wide, scalable through its `viewBox`. Medium error correction (15 %).
pub(super) fn qr_svg(text: &str) -> AppResult<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::InvalidInput(
            "Nothing to encode in the QR code".into(),
        ));
    }
    if text.len() > MAX_QR_TEXT {
        return Err(AppError::InvalidInput(format!(
            "Text too long for a QR code (more than {MAX_QR_TEXT} bytes)"
        )));
    }
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|e| AppError::InvalidInput(format!("Couldn't make a QR code: {e}")))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_become_svg() {
        let svg = qr_svg(" http://192.168.1.20:8042/opds ").unwrap();
        assert!(
            svg.contains("<svg") && svg.contains("viewBox") && svg.ends_with("</svg>"),
            "{svg}"
        );
        assert!(svg.contains("#000000") && svg.contains("#ffffff"));
        // Deterministic: the same URL gives the same picture.
        assert_eq!(svg, qr_svg("http://192.168.1.20:8042/opds").unwrap());
        assert_ne!(svg, qr_svg("http://192.168.1.21:8042/opds").unwrap());
    }

    #[test]
    fn empty_or_oversized_text_is_refused() {
        assert!(matches!(qr_svg("  "), Err(AppError::InvalidInput(_))));
        assert!(matches!(
            qr_svg(&"x".repeat(MAX_QR_TEXT + 1)),
            Err(AppError::InvalidInput(_))
        ));
        assert!(qr_svg(&"x".repeat(MAX_QR_TEXT)).is_ok());
    }
}
