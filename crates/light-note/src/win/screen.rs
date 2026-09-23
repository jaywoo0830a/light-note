//! The elm screen: text, buttons and panels.
//!
//! This is the half of the UI that `elm-magic` is good at — structure, state and
//! events, with no styling (the Windows adapter does not carry a style layer, by
//! design).  Everything it renders comes from [`ViewModel`] props, and every
//! button it owns reports an [`Intent`] back to the host; the screen never
//! touches the document, the workers or the frame clock.
//!
//! ```text
//! host (owns everything)  ──props──►  Screen  ──Intent──►  host
//! ```
//!
//! The drawing surface is *not* here: it needs WinUI controls (a `Canvas` with
//! two `Image` layers and live shapes), so the host builds it directly — the
//! split the adapter's examples describe (host state + callback props for the
//! text, `<Raw>`/host controls for the platform surface).

use super::Intent;
use crate::ink::Tool;

elm_magic::view! {
    pub fn Screen(
        title: String = String::new(),
        subtitle: String = String::new(),
        tool_line: String = String::new(),
        pages: usize = 1,
        page_index: usize = 0,
        strokes: usize = 0,
        zoom: f32 = 100.0,
        tablet: String = String::new(),
        frame: String = String::new(),
        hint: String = String::new(),
        help: bool = false,
        dev: bool = false,
        dev_report: String = String::new(),
        on_intent: fn(Intent),
    ) {
        <Col>
            <Strong>"{title}"</Strong>
            "{subtitle}"

            <Row>
                <Button on_click={on_intent(Intent::Tool(Tool::Pen))}>"Pen"</Button>
                <Button on_click={on_intent(Intent::Tool(Tool::Highlighter))}>"Highlighter"</Button>
                <Button on_click={on_intent(Intent::Tool(Tool::Eraser))}>"Eraser"</Button>
                <Button on_click={on_intent(Intent::Thinner)}>"Thinner"</Button>
                <Button on_click={on_intent(Intent::Thicker)}>"Thicker"</Button>
            </Row>

            <Row>
                <Button on_click={on_intent(Intent::Undo)}>"Undo"</Button>
                <Button on_click={on_intent(Intent::Redo)}>"Redo"</Button>
                <Button on_click={on_intent(Intent::ClearPage)}>"Clear page"</Button>
                <Button on_click={on_intent(Intent::OpenPdf)}>"Open PDF"</Button>
                <Button on_click={on_intent(Intent::ExportPdf)}>"Save PDF"</Button>
                <Button on_click={on_intent(Intent::ExportPng)}>"Save PNG"</Button>
            </Row>

            <Row>
                <Button on_click={on_intent(Intent::PreviousPage)}>"Prev"</Button>
                <Button on_click={on_intent(Intent::NextPage)}>"Next"</Button>
                <Button on_click={on_intent(Intent::ZoomOut)}>"Zoom -"</Button>
                <Button on_click={on_intent(Intent::ZoomIn)}>"Zoom +"</Button>
                <Button on_click={on_intent(Intent::ZoomReset)}>"100%"</Button>
                <Button on_click={on_intent(Intent::SetRefresh(0))}>"Auto Hz"</Button>
                <Button on_click={on_intent(Intent::SetRefresh(60))}>"60 Hz"</Button>
                <Button on_click={on_intent(Intent::SetRefresh(120))}>"120 Hz"</Button>
                <Button on_click={on_intent(Intent::SetRefresh(180))}>"180 Hz"</Button>
                <Button on_click={on_intent(Intent::SetRefresh(240))}>"240 Hz"</Button>
            </Row>

            "{tool_line}"
            <Row>
                "{pages} page(s) · page {page_index} · {strokes} strokes · zoom {zoom}%"
            </Row>
            "{tablet}"
            "{frame}"

            <Row>
                <Button on_click={on_intent(Intent::ToggleHelp)}>"Help"</Button>
                <Button on_click={on_intent(Intent::ToggleDev)}>"Diagnostics"</Button>
                <Button on_click={on_intent(Intent::RescanTablet)}>"Rescan tablet"</Button>
                <Button on_click={on_intent(Intent::Quit)}>"Quit"</Button>
            </Row>

            <If when={help}>
                <Col>
                    <Strong>"Shortcuts and gestures"</Strong>
                    "{hint}"
                </Col>
            </If>

            <If when={dev}>
                <Col>
                    <Strong>"Diagnostics"</Strong>
                    "{dev_report}"
                </Col>
            </If>
        </Col>
    }
}
