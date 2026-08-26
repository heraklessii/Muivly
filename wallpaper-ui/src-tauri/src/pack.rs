//! The `.muivly` wallpaper package: one file to hand somebody.
//!
//! A wallpaper is a video, a name, and — if the author bothered — a preview
//! image and a credit. Sending that as a bare `.mp4` loses everything but the
//! video, so a package is a zip holding the media plus a small `manifest.json`
//! saying what it is and who made it.
//!
//! ## Why zip, and why by hand
//!
//! Zip because every user on Windows can open one without installing
//! anything, which matters for a format meant to be passed around.
//!
//! By hand because the alternative is a compression library, and there is
//! nothing here to compress: the payload is an H.264 file, which is already
//! about as small as it is going to get. Every entry is written with the
//! "stored" method — no deflate, no compression dependency, and unpacking is
//! a copy. What that costs is roughly two hundred lines; what it saves is a
//! dependency tree, its build time and its share of the binary.
//!
//! Nothing here holds a whole wallpaper in memory. A 300 MB clip is read and
//! written in 64 KB pieces, because the one thing this project will not do is
//! spend a third of a gigabyte to move a file.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How much of a file moves at a time.
const CHUNK: usize = 64 * 1024;

/// A package larger than this is refused on import. Not a technical limit —
/// it is a sanity check against a zip bomb or a mistyped file, and 4 GB is
/// already several times larger than any real wallpaper.
const MAX_PACKAGE: u64 = 4 * 1024 * 1024 * 1024;

/// The name every package's description is stored under.
const MANIFEST: &str = "manifest.json";

/// What a package says about itself.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub author: String,
    /// The entry inside the package that is the wallpaper itself.
    pub file: String,
    /// An optional still, shown in the library before the video is opened.
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub license: String,
    /// Where it came from, if anywhere — a site, a workshop page.
    #[serde(default)]
    pub source: String,
}

/// What an import produced.
#[derive(Serialize)]
pub struct Imported {
    pub path: String,
    pub title: String,
    pub author: String,
    pub preview: Option<String>,
}

/// Write one wallpaper out as a package.
#[tauri::command(async)]
pub fn export_package(
    source: String,
    destination: String,
    name: String,
    author: String,
    license: String,
) -> Result<(), String> {
    let source = PathBuf::from(&source);
    if !source.is_file() {
        return Err("that wallpaper is not on disk any more".to_string());
    }

    // The entry keeps the original extension: the engine picks its decoder
    // from it, and a package that unpacks to a file called "wallpaper" with
    // no extension would not play.
    let extension = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "mp4".to_string());
    let entry = format!("wallpaper.{extension}");

    let manifest = Manifest {
        name: if name.trim().is_empty() {
            source
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            name
        },
        author,
        file: entry.clone(),
        preview: None,
        license,
        source: String::new(),
    };

    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;

    let out = File::create(&destination).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(BufWriter::new(out));
    zip.add_bytes(MANIFEST, &json)?;
    zip.add_file(&entry, &source)?;
    zip.finish()
}

/// Unpack a package into the wallpapers folder.
///
/// Returns where the wallpaper landed, so the caller can add it to the
/// library — the frontend owns the library, and this owns the disk.
#[tauri::command(async)]
pub fn import_package(package: String) -> Result<Imported, String> {
    let package = PathBuf::from(&package);
    let size = std::fs::metadata(&package)
        .map_err(|e| e.to_string())?
        .len();
    if size > MAX_PACKAGE {
        return Err("that package is implausibly large".to_string());
    }

    let mut zip = ZipReader::open(&package)?;

    let manifest: Manifest = serde_json::from_slice(&zip.read_entry(MANIFEST)?)
        .map_err(|e| format!("the package has no readable manifest: {e}"))?;

    let folder = crate::web::wallpapers_dir()?.join(safe_name(&manifest.name));
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    let wallpaper = folder.join(safe_name(&manifest.file));
    zip.extract(&manifest.file, &wallpaper)?;

    let preview = match &manifest.preview {
        Some(name) => {
            let path = folder.join(safe_name(name));
            zip.extract(name, &path).ok().map(|()| path)
        }
        None => None,
    };

    Ok(Imported {
        path: wallpaper.display().to_string(),
        title: manifest.name,
        author: manifest.author,
        preview: preview.map(|p| p.display().to_string()),
    })
}

/// What a package holds, without unpacking it. For a confirmation step
/// before anything is written to disk.
#[tauri::command(async)]
pub fn inspect_package(package: String) -> Result<Manifest, String> {
    let mut zip = ZipReader::open(Path::new(&package))?;
    serde_json::from_slice(&zip.read_entry(MANIFEST)?)
        .map_err(|e| format!("the package has no readable manifest: {e}"))
}

/// Strip a name down to something that can be a file on Windows.
///
/// This is the security boundary of the whole format: an entry named
/// `..\..\Startup\evil.exe` would otherwise be written wherever the path led.
/// Everything that could steer a path — separators, drive colons, the parent
/// directory — is removed rather than escaped, because a wallpaper package
/// has no business naming a directory at all.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();

    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "wallpaper".to_string()
    } else {
        cleaned
    }
}

// ---------------------------------------------------------------------------
// The zip itself
// ---------------------------------------------------------------------------

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL: u32 = 0x0605_4b50;
/// "Stored", which is to say not compressed.
const STORED: u16 = 0;
/// The version needed to extract: 2.0, which is what stored entries have
/// wanted since 1993.
const VERSION: u16 = 20;

/// One entry, as the central directory will need to describe it.
struct Entry {
    name: String,
    crc: u32,
    size: u64,
    offset: u64,
}

struct ZipWriter<W: Write + Seek> {
    out: W,
    entries: Vec<Entry>,
    at: u64,
}

impl<W: Write + Seek> ZipWriter<W> {
    fn new(out: W) -> Self {
        Self {
            out,
            entries: Vec::new(),
            at: 0,
        }
    }

    fn add_bytes(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        let crc = crc32(data);
        self.write_local(name, crc, data.len() as u64)?;
        self.write_all(data)
    }

    /// Copy a file in, without ever holding it in memory.
    ///
    /// Read twice: once for the checksum, once for the bytes. A zip's local
    /// header carries the CRC *before* the data, so the alternative is either
    /// buffering the whole file or writing a data descriptor after it — and
    /// data descriptors are the corner of the format that other tools get
    /// wrong. A second pass over a file the OS has just cached is cheap; a
    /// package that some unzip program refuses is not.
    fn add_file(&mut self, name: &str, path: &Path) -> Result<(), String> {
        let (crc, size) = checksum_of(path)?;
        self.write_local(name, crc, size)?;

        let mut file = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
        let mut buffer = vec![0u8; CHUNK];
        loop {
            let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            self.write_all(&buffer[..read])?;
        }

        Ok(())
    }

    fn write_local(&mut self, name: &str, crc: u32, size: u64) -> Result<(), String> {
        let offset = self.at;
        let bytes = name.as_bytes();

        let mut header = Vec::with_capacity(30 + bytes.len());
        header.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
        header.extend_from_slice(&VERSION.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes()); // flags
        header.extend_from_slice(&STORED.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes()); // modification time
        header.extend_from_slice(&0u16.to_le_bytes()); // modification date
        header.extend_from_slice(&crc.to_le_bytes());
        header.extend_from_slice(&(size as u32).to_le_bytes());
        header.extend_from_slice(&(size as u32).to_le_bytes());
        header.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        header.extend_from_slice(bytes);

        self.write_all(&header)?;
        self.entries.push(Entry {
            name: name.to_string(),
            crc,
            size,
            offset,
        });
        Ok(())
    }

    fn write_all(&mut self, data: &[u8]) -> Result<(), String> {
        self.out.write_all(data).map_err(|e| e.to_string())?;
        self.at += data.len() as u64;
        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        let directory_at = self.at;

        let mut directory = Vec::new();
        for entry in &self.entries {
            let bytes = entry.name.as_bytes();
            directory.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
            directory.extend_from_slice(&VERSION.to_le_bytes()); // made by
            directory.extend_from_slice(&VERSION.to_le_bytes()); // needed
            directory.extend_from_slice(&0u16.to_le_bytes()); // flags
            directory.extend_from_slice(&STORED.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes()); // time
            directory.extend_from_slice(&0u16.to_le_bytes()); // date
            directory.extend_from_slice(&entry.crc.to_le_bytes());
            directory.extend_from_slice(&(entry.size as u32).to_le_bytes());
            directory.extend_from_slice(&(entry.size as u32).to_le_bytes());
            directory.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes()); // extra
            directory.extend_from_slice(&0u16.to_le_bytes()); // comment
            directory.extend_from_slice(&0u16.to_le_bytes()); // disk number
            directory.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            directory.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            directory.extend_from_slice(&(entry.offset as u32).to_le_bytes());
            directory.extend_from_slice(bytes);
        }

        let count = self.entries.len() as u16;
        let size = directory.len() as u32;
        self.write_all(&directory)?;

        let mut end = Vec::with_capacity(22);
        end.extend_from_slice(&END_OF_CENTRAL.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes()); // this disk
        end.extend_from_slice(&0u16.to_le_bytes()); // disk with the directory
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&size.to_le_bytes());
        end.extend_from_slice(&(directory_at as u32).to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes()); // comment length
        self.write_all(&end)?;

        self.out.flush().map_err(|e| e.to_string())
    }
}

/// Where one entry's data starts and how long it is.
struct Located {
    at: u64,
    size: u64,
    crc: u32,
}

struct ZipReader {
    file: BufReader<File>,
    directory: Vec<(String, u64, u64, u32)>,
}

impl ZipReader {
    fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("cannot open the package: {e}"))?;
        let mut file = BufReader::new(file);
        let directory = read_directory(&mut file)?;
        Ok(Self { file, directory })
    }

    /// Where an entry's bytes actually begin, which is past a local header
    /// whose name and extra field can be any length.
    fn locate(&mut self, name: &str) -> Result<Located, String> {
        let (_, offset, size, crc) = self
            .directory
            .iter()
            .find(|(entry, _, _, _)| entry == name)
            .ok_or_else(|| format!("the package has no {name}"))?;
        let (offset, size, crc) = (*offset, *size, *crc);

        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;

        let mut header = [0u8; 30];
        self.file
            .read_exact(&mut header)
            .map_err(|e| e.to_string())?;
        if u32::from_le_bytes(header[0..4].try_into().unwrap()) != LOCAL_HEADER {
            return Err("the package is damaged".to_string());
        }
        if u16::from_le_bytes(header[8..10].try_into().unwrap()) != STORED {
            return Err("this package is compressed, which Muivly does not write".to_string());
        }

        let name_length = u16::from_le_bytes(header[26..28].try_into().unwrap()) as u64;
        let extra_length = u16::from_le_bytes(header[28..30].try_into().unwrap()) as u64;

        Ok(Located {
            at: offset + 30 + name_length + extra_length,
            size,
            crc,
        })
    }

    /// A whole entry in memory. Only ever used for the manifest, which is a
    /// few hundred bytes.
    fn read_entry(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let found = self.locate(name)?;
        if found.size > 1024 * 1024 {
            return Err(format!("{name} is far too large to be a manifest"));
        }

        self.file
            .seek(SeekFrom::Start(found.at))
            .map_err(|e| e.to_string())?;
        let mut data = vec![0u8; found.size as usize];
        self.file.read_exact(&mut data).map_err(|e| e.to_string())?;

        if crc32(&data) != found.crc {
            return Err(format!("{name} is damaged"));
        }
        Ok(data)
    }

    /// Copy one entry out to a file, in pieces, checking it as it goes.
    fn extract(&mut self, name: &str, to: &Path) -> Result<(), String> {
        let found = self.locate(name)?;
        self.file
            .seek(SeekFrom::Start(found.at))
            .map_err(|e| e.to_string())?;

        let mut out = BufWriter::new(File::create(to).map_err(|e| e.to_string())?);
        let mut buffer = vec![0u8; CHUNK];
        let mut left = found.size;
        let mut running = Crc::new();

        while left > 0 {
            let want = CHUNK.min(left as usize);
            self.file
                .read_exact(&mut buffer[..want])
                .map_err(|e| e.to_string())?;
            running.update(&buffer[..want]);
            out.write_all(&buffer[..want]).map_err(|e| e.to_string())?;
            left -= want as u64;
        }

        out.flush().map_err(|e| e.to_string())?;

        if running.finish() != found.crc {
            // Removed rather than left behind: a half-written wallpaper in
            // the library is worse than a failed import.
            let _ = std::fs::remove_file(to);
            return Err(format!("{name} is damaged"));
        }

        Ok(())
    }
}

/// The central directory, as (name, local header offset, size, crc).
///
/// Read from the end of the file, which is where zip keeps its index — and
/// the reason a zip can be appended to without being rewritten.
fn read_directory(file: &mut BufReader<File>) -> Result<Vec<(String, u64, u64, u32)>, String> {
    let length = file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if length < 22 {
        return Err("that file is too small to be a package".to_string());
    }

    // The end record is 22 bytes plus a comment of up to 64 KB, so the tail
    // is searched backwards for its signature.
    let tail_length = (length).min(22 + 0xFFFF);
    file.seek(SeekFrom::End(-(tail_length as i64)))
        .map_err(|e| e.to_string())?;
    let mut tail = vec![0u8; tail_length as usize];
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;

    let end = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&i| u32::from_le_bytes(tail[i..i + 4].try_into().unwrap()) == END_OF_CENTRAL)
        .ok_or("that file is not a package")?;

    let count = u16::from_le_bytes(tail[end + 10..end + 12].try_into().unwrap()) as usize;
    let at = u32::from_le_bytes(tail[end + 16..end + 20].try_into().unwrap()) as u64;

    file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;

    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let mut header = [0u8; 46];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        if u32::from_le_bytes(header[0..4].try_into().unwrap()) != CENTRAL_HEADER {
            return Err("the package index is damaged".to_string());
        }

        let crc = u32::from_le_bytes(header[16..20].try_into().unwrap());
        let size = u32::from_le_bytes(header[24..28].try_into().unwrap()) as u64;
        let name_length = u16::from_le_bytes(header[28..30].try_into().unwrap()) as usize;
        let extra_length = u16::from_le_bytes(header[30..32].try_into().unwrap()) as usize;
        let comment_length = u16::from_le_bytes(header[32..34].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(header[42..46].try_into().unwrap()) as u64;

        let mut name = vec![0u8; name_length];
        file.read_exact(&mut name).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Current((extra_length + comment_length) as i64))
            .map_err(|e| e.to_string())?;

        entries.push((
            String::from_utf8_lossy(&name).into_owned(),
            offset,
            size,
            crc,
        ));
    }

    Ok(entries)
}

/// A file's CRC and its length, read in pieces.
fn checksum_of(path: &Path) -> Result<(u32, u64), String> {
    let mut file = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let mut buffer = vec![0u8; CHUNK];
    let mut crc = Crc::new();
    let mut size = 0u64;

    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        crc.update(&buffer[..read]);
        size += read as u64;
    }

    Ok((crc.finish(), size))
}

/// CRC-32, the one zip has used since the beginning.
///
/// Bitwise rather than table-driven: the table is 1 KB that would have to be
/// built at startup or embedded, and this runs over a few hundred megabytes
/// once per package — an operation already bounded by the disk.
struct Crc(u32);

impl Crc {
    fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    fn update(&mut self, data: &[u8]) {
        for byte in data {
            self.0 ^= *byte as u32;
            for _ in 0..8 {
                // The reversed polynomial 0x04C11DB7, which is how CRC-32
                // is written when the bits run the other way.
                let mask = (self.0 & 1).wrapping_neg();
                self.0 = (self.0 >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        !self.0
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(data);
    crc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check value every CRC-32 implementation is measured against.
    #[test]
    fn crc_matches_the_known_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn crc_of_nothing_is_zero() {
        assert_eq!(crc32(b""), 0);
    }

    /// The one that matters: a package naming an entry outside the folder it
    /// is being unpacked into must not be able to write there.
    #[test]
    fn a_traversing_name_cannot_escape() {
        // The property, not the exact spelling: whatever comes out has to be
        // one file name, in the folder it was given, with nothing in it that
        // Windows would read as a path.
        for hostile in [
            r"..\..\Startup\evil.exe",
            "../../etc/passwd",
            r"C:\Windows\system32\x.dll",
            "..",
        ] {
            let safe = safe_name(hostile);
            assert!(
                !safe.contains(['\\', '/', ':']),
                "{hostile:?} became {safe:?}"
            );
            assert!(!safe.starts_with('.'), "{hostile:?} became {safe:?}");

            let folder = Path::new(r"C:\wallpapers");
            let joined = folder.join(&safe);
            assert_eq!(joined.parent(), Some(folder), "{hostile:?} escaped");
        }
    }

    #[test]
    fn a_name_of_only_dots_becomes_something_writable() {
        assert_eq!(safe_name(".."), "wallpaper");
        assert_eq!(safe_name("   "), "wallpaper");
        assert_eq!(safe_name(""), "wallpaper");
    }

    #[test]
    fn an_ordinary_name_is_left_alone() {
        assert_eq!(safe_name("Neon Rain.mp4"), "Neon Rain.mp4");
    }

    #[test]
    fn a_package_round_trips() {
        let dir = std::env::temp_dir().join(format!("muivly-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let source = dir.join("clip.mp4");
        // Larger than one chunk, so the piece-by-piece paths are the ones
        // being tested rather than a single short read.
        let payload: Vec<u8> = (0..CHUNK * 3 + 17).map(|i| (i % 251) as u8).collect();
        std::fs::write(&source, &payload).unwrap();

        let package = dir.join("test.muivly");
        let out = File::create(&package).unwrap();
        let mut zip = ZipWriter::new(BufWriter::new(out));
        let manifest = serde_json::to_vec(&Manifest {
            name: "Test".to_string(),
            file: "wallpaper.mp4".to_string(),
            ..Default::default()
        })
        .unwrap();
        zip.add_bytes(MANIFEST, &manifest).unwrap();
        zip.add_file("wallpaper.mp4", &source).unwrap();
        zip.finish().unwrap();

        let mut read = ZipReader::open(&package).unwrap();
        let back: Manifest = serde_json::from_slice(&read.read_entry(MANIFEST).unwrap()).unwrap();
        assert_eq!(back.name, "Test");

        let extracted = dir.join("out.mp4");
        read.extract("wallpaper.mp4", &extracted).unwrap();
        assert_eq!(std::fs::read(&extracted).unwrap(), payload);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_package_is_refused() {
        let dir = std::env::temp_dir().join(format!("muivly-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("not-a-package.muivly");
        std::fs::write(&path, b"this is just some text, at some length").unwrap();

        assert!(ZipReader::open(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
