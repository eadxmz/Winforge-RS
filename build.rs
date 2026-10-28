fn main() {
    slint_build::compile("ui/main.slint").expect("Failed to compile Slint UI");
    embed_resource::compile("app.rc", embed_resource::NONE);
}
