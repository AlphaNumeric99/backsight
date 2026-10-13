fn main() {
    // Tauri only embeds the built frontend when the `tauri` crate has its `custom-protocol`
    // feature on, and the Tauri CLI is what turns that on. A plain `cargo build --release`
    // therefore produces an app that looks finished but points its window at the dev server,
    // so on any machine that isn't running `npm run dev` it opens on "localhost refused to
    // connect". Fail here, where the message can say what to run instead.
    if tauri_build::is_dev() && std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!(
            "this release build would have no frontend in it, and would open on \
             \"localhost refused to connect\": the `tauri` crate was compiled without its \
             `custom-protocol` feature, which leaves the window pointing at `devUrl`. \
             Build the app with `npm run tauri build` from `app/` (`--no-bundle` for just \
             the executable), or add `--features tauri/custom-protocol` to this cargo command."
        );
    }

    tauri_build::build()
}
