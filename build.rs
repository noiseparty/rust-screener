fn main() {
    // Exe icon (Explorer, taskbar when run in a classic console window).
    embed_resource::compile("assets/icon.rc", embed_resource::NONE).manifest_optional().unwrap();
}
