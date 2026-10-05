#[cfg(windows)]
fn main() {
    winresource::WindowsResource::new()
        .set_icon("assets/icon.ico")
        .compile()
        .unwrap();
}

#[cfg(not(windows))]
fn main() {}
