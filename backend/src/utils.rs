use std::path::Path;
use std::{
    fs,
    path::PathBuf,
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};

use opencv::{core::ToInputArray, imgcodecs::imwrite_def};

static DATASET_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    // Store in the project root so it survives `dx build` wipes of the
    // target directory. env!("CARGO_MANIFEST_DIR") = <project>/backend,
    // parent = <project>.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("dataset");
    fs::create_dir_all(dir.clone()).unwrap();
    dir
});

#[cfg(debug_assertions)]
static DATASET_RECORDINGS_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let dir = DATASET_DIR.join("recordings");
    fs::create_dir_all(dir.clone()).unwrap();
    dir
});

#[cfg(debug_assertions)]
static DATASET_RUNE_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let dir = DATASET_DIR.join("rune");
    fs::create_dir_all(dir.clone()).unwrap();
    dir
});

#[derive(Debug)]
pub enum DatasetDir {
    Root,
    #[cfg(debug_assertions)]
    Recordings,
    #[cfg(debug_assertions)]
    Rune,
}

impl DatasetDir {
    pub fn to_folder(&self) -> PathBuf {
        match self {
            DatasetDir::Root => DATASET_DIR.clone(),
            #[cfg(debug_assertions)]
            DatasetDir::Recordings => DATASET_RECORDINGS_DIR.clone(),
            #[cfg(debug_assertions)]
            DatasetDir::Rune => DATASET_RUNE_DIR.clone(),
        }
    }
}

pub fn save_image_to_default(mat: &impl ToInputArray, dir: DatasetDir) {
    save_image_to(mat, dir, format!("{}.png", epoch_millis_as_string()));
}

pub fn save_image_to<P: AsRef<Path>>(mat: &impl ToInputArray, dir: DatasetDir, relative: P) {
    let folder = dir.to_folder();
    let mut image = folder.join(relative);
    if image.extension().is_none_or(|ext| ext != "png") {
        image = image.join(format!("{}.png", epoch_millis_as_string()));
    }

    if !image.exists() {
        fs::create_dir_all(image.parent().unwrap()).unwrap();
    }

    let _ = imwrite_def(image.to_str().unwrap(), mat);
}

#[cfg(debug_assertions)]
pub fn save_file_to<P: AsRef<Path>, C: AsRef<[u8]>>(contents: C, dir: DatasetDir, relative: P) {
    let folder = dir.to_folder();
    let file = folder.join(relative);
    if !file.exists() {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
    }

    let _ = fs::write(file, contents);
}

fn epoch_millis_as_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}
