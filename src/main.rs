//! Sonora — a small native macOS tool for trimming audio and making it louder.

use std::path::PathBuf;

use gpui::Application;
// The engine lives in the library crate; importing it here lets the UI refer
// to it as `crate::audio`.
use sonora::audio;

mod ui;

fn main() {
    // Paths given on the command line (Launch Services' legacy `-psn_` argument
    // is ignored).
    let initial: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|arg| !arg.to_string_lossy().starts_with("-psn_"))
        .map(PathBuf::from)
        .collect();

    // Finder can deliver "open" requests before the window exists; they queue
    // here until the window is ready to take them.
    let (open_tx, open_rx) = ui::mailbox::channel::<Vec<String>>();

    let app = Application::new().with_assets(ui::assets::Assets);
    app.on_open_urls(move |urls| open_tx.send(urls));
    app.run(move |cx| {
        ui::init(cx);
        cx.activate(true);
        if let Err(err) = ui::open_main_window(cx, initial, open_rx) {
            eprintln!("Sonora couldn't open its window: {err:#}");
            cx.quit();
        }
    });
}
