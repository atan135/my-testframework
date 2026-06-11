use std::path::{Path, PathBuf};

use tower_http::services::{ServeDir, ServeFile};

pub(crate) fn default_client_dist_path() -> PathBuf {
    let exe_relative_path = std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("client").join("dist"))
    });

    if let Some(path) = exe_relative_path
        && path.exists()
    {
        return path;
    }

    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("client")
        .join("dist")
}

pub(crate) fn static_service(client_dist_path: PathBuf) -> ServeDir<ServeFile> {
    let index_path = client_dist_path.join("index.html");
    ServeDir::new(client_dist_path).fallback(ServeFile::new(index_path))
}
