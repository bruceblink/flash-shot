fn main() {
    #[cfg(target_os = "windows")]
    {
        let icon = std::path::Path::new("resources/icons/icon.ico");
        if icon.exists() {
            let mut resource = winres::WindowsResource::new();
            resource.set_icon(icon.to_string_lossy().as_ref());
            resource
                .compile()
                .expect("failed to compile Windows resources");
        }
    }
}
