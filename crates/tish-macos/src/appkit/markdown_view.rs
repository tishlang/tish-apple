//! Read-only `NSTextView` content from Markdown via `NSAttributedString` (macOS 12+).

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSColor, NSFont, NSTextView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    NSAttributedString, NSAttributedStringMarkdownInterpretedSyntax,
    NSAttributedStringMarkdownParsingFailurePolicy, NSAttributedStringMarkdownParsingOptions,
    NSString,
};

pub(crate) fn attributed_string_from_markdown(
    _mtm: MainThreadMarker,
    markdown: &str,
) -> Retained<NSAttributedString> {
    let opts = NSAttributedStringMarkdownParsingOptions::new();
    opts.setFailurePolicy(NSAttributedStringMarkdownParsingFailurePolicy::ReturnPartiallyParsedIfPossible);
    opts.setInterpretedSyntax(NSAttributedStringMarkdownInterpretedSyntax::Full);
    let ns_md = NSString::from_str(markdown);
    let allocated = NSAttributedString::alloc();
    match NSAttributedString::initWithMarkdownString_options_baseURL_error(
        allocated,
        &ns_md,
        Some(&opts),
        None,
    ) {
        Ok(a) => a,
        Err(_) => NSAttributedString::from_nsstring(&ns_md),
    }
}

pub(crate) fn set_text_view_markdown(tv: &NSTextView, markdown: &str, mtm: MainThreadMarker) {
    tv.setRichText(true);
    tv.setEditable(false);
    tv.setSelectable(true);
    let attr = attributed_string_from_markdown(mtm, markdown);
    unsafe {
        if let Some(ts) = tv.textStorage() {
            ts.setAttributedString(&attr);
        }
    }
}

pub(crate) fn apply_markdown_text_view_chrome(tv: &NSTextView, font_size: CGFloat) {
    let fg = NSColor::labelColor();
    tv.setTextColor(Some(&fg));
    tv.setInsertionPointColor(Some(&fg));
    let font = NSFont::systemFontOfSize(font_size);
    tv.setFont(Some(&font));
}
