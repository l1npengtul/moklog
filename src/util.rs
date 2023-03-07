

pub struct Empty {}

impl AsRef<[u8]> for Empty {
    fn as_ref(&self) -> &[u8] {
        const NOTHING: &[u8] = &[];
        NOTHING
    }
}

#[macro_export]
macro_rules! mmap_load {
    ($path:expr) => {{
        let a: Box<impl AsRef<[u8]>> = match unsafe { MmapOptions::new().map(path) } {
            Ok(a) => Box::new(a),
            Err(_) => Box::new(Empty {}),
        };
        a
    }};
}

#[macro_export]
macro_rules! walker {
        ($dir:expr) => {{
            let w = WalkBuilder::new($dir)
                .ignore(true)
                .add_custom_ignore_filename(".mkignore");
            w
        }};
    }
