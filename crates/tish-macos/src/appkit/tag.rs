//! Canonical host tag names (legacy + PascalCase aliases map to one dispatch key).

/// Normalize vnode tag strings so `ScrollView` / `scrollable` share one implementation key.
pub(super) fn canonical_host_tag(tag: &str) -> &str {
    match tag {
        "SidebarWindow" | "sidebar_window" => "sidebar_window",
        "Window" | "macos_window" => "macos_window",
        "ScrollView" | "scrollable" => "scrollable",
        "GroupedTable" | "grouped_table" => "grouped_table",
        "Section" | "section" => "section",
        "Text" | "text" | "p" | "span" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "title"
        | "heading" => "text",
        "Row" | "HStack" | "row" | "hstack" | "horizontal" => "row",
        "Column" | "column" | "VStack" | "vstack" => "column",
        "ZStack" | "zstack" => "zstack",
        "Split" | "split" => "split",
        "VisualEffect" | "visual_effect" => "visual_effect",
        "PickList" | "pick_list" | "picklist" | "select" => "pick_list",
        "TextInput" | "textinput" | "text-input" | "input" => "textinput",
        "TextEditor" | "text_editor" | "textarea" => "text_editor",
        "MarkdownText" | "markdown_text" => "markdown_text",
        "ProgressBar" | "progress_bar" | "progressbar" => "progress_bar",
        "Image" | "image" => "image",
        "PasswordField" | "password" | "secure" => "password",
        "Switch" | "toggler" | "toggle" | "switch" => "toggler",
        "TabView" | "tabs" | "tabview" => "tabs",
        "WebView" | "webview" => "webview",
        "List" | "list" | "table" => "list",
        "Tooltip" | "tooltip" => "tooltip",
        "Space" | "space" => "space",
        "Button" | "button" => "button",
        "Checkbox" | "checkbox" => "checkbox",
        "Slider" | "slider" => "slider",
        "Radio" | "radio" => "radio",
        "Rule" | "rule" | "separator" | "divider" => "rule",
        "Tab" | "tab" => "tab",
        _ => tag,
    }
}
