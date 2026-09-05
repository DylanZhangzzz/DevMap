use std::sync::OnceLock;

pub const DOCK_RESOURCE_URI: &str = "ui://devmap/dock/v1.html";
pub const DOCK_MIME_TYPE: &str = "text/html;profile=mcp-app";

pub fn dock_html() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    const PLACEHOLDER: &str = "/* DEVMAP_METRO_CORE */";
    const TEMPLATE: &str = include_str!("../assets/dock.html");
    const CORE: &str = include_str!("../assets/metro-core.js");

    HTML.get_or_init(|| {
        assert_eq!(
            TEMPLATE.matches(PLACEHOLDER).count(),
            1,
            "Dock template must contain one metro core placeholder"
        );
        assert!(
            !CORE.to_ascii_lowercase().contains("</script"),
            "Metro core must remain safe inside the Dock script resource"
        );
        // The template has no preformatted text or multiline string literals.
        // Strip source indentation for transport, keeping readable source and the
        // 148 KiB resource budget including persistent journey navigation.
        // Keep newlines for JavaScript comments.
        TEMPLATE
            .lines()
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n")
            .replacen(
                PLACEHOLDER,
                &CORE
                    .lines()
                    .map(str::trim_start)
                    .collect::<Vec<_>>()
                    .join("\n"),
                1,
            )
    })
    .as_str()
}
