use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub(crate) struct Fixture {
    pub root: PathBuf,
    pub theme: Theme,
}
impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "omatainer-theme-reload-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("fonts")).unwrap();
        fs::create_dir(root.join("cache")).unwrap();
        let defaults = FontDefinitions::default();
        for (key, file) in [("Hack", "hack.ttf"), ("Ubuntu-Light", "ubuntu.ttf")] {
            fs::write(root.join("fonts").join(file), &defaults.font_data[key].font).unwrap();
        }
        let mut theme = Theme::default();
        theme.path = root.join("colors.toml");
        let fixture = Self { root, theme };
        fixture.colors("background = '#010203'\naccent = '#405060'\n");
        fixture.shell("[font]\nbase-size = 12\n");
        fixture.select("Hack", "hack.ttf");
        fixture
    }
    pub fn colors(&self, text: &str) {
        fs::write(&self.theme.path, text).unwrap();
    }
    pub fn shell(&self, text: &str) {
        fs::write(self.root.join("shell.toml"), text).unwrap();
    }
    pub fn select(&self, family: &str, file: &str) {
        fs::write(
            self.root.join("selected-font"),
            format!(
                "{family}\n{}\n0\n",
                self.root.join("fonts").join(file).display()
            ),
        )
        .unwrap();
    }
    pub fn quiet_loader(&self) -> Loader {
        Loader::with_resolver(self.theme.clone(), self.resolver(), Duration::from_secs(30)).unwrap()
    }
    pub fn loader(&self) -> Loader {
        Loader::with_resolver(
            self.theme.clone(),
            self.resolver(),
            Duration::from_millis(15),
        )
        .unwrap()
    }
    pub(crate) fn installed_loader(&self, family: &str) -> Loader {
        Loader::with_resolver(
            self.theme.clone(),
            self.fontconfig(family),
            Duration::from_millis(20),
        )
        .unwrap()
    }
    pub(super) fn resolver(&self) -> FileResolver {
        FileResolver(self.root.join("selected-font"))
    }
    pub(super) fn fontconfig(&self, family: &str) -> Fontconfig {
        self.write_fontconfig(family);
        let mut resolver = Fontconfig::default();
        resolver.environment = vec![
            (
                "FONTCONFIG_FILE".into(),
                self.root.join("fonts.conf").to_string_lossy().into(),
            ),
            ("FONTCONFIG_PATH".into(), self.root.to_string_lossy().into()),
        ];
        resolver
    }
    pub(crate) fn write_fontconfig(&self, family: &str) {
        fs::write(self.root.join("fonts.conf"), format!(r#"<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd"><fontconfig><dir>{}/fonts</dir><cachedir>{}/cache</cachedir><match target="pattern"><test name="family" qual="any"><string>monospace</string></test><edit name="family" mode="prepend_first" binding="strong"><string>{family}</string></edit></match></fontconfig>"#, self.root.display(), self.root.display())).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
pub(super) struct FileResolver(PathBuf);
impl Resolver for FileResolver {
    fn resolve(&mut self) -> Result<FontSource, String> {
        let raw = read_text(&self.0)?;
        let fields: Vec<_> = raw.lines().collect();
        if fields.len() != 3 {
            return Err("invalid private font selection".into());
        }
        Ok(FontSource {
            family: fields[0].into(),
            path: fields[1].into(),
            index: fields[2].parse().map_err(|_| "invalid face index")?,
        })
    }
}
pub(crate) struct Control {
    pub started: Receiver<()>,
    pub release: Sender<()>,
}
struct Held {
    inner: FileResolver,
    started: Sender<()>,
    release: Receiver<()>,
}
impl Resolver for Held {
    fn resolve(&mut self) -> Result<FontSource, String> {
        self.started.send(()).map_err(|_| "fixture closed")?;
        self.release.recv().map_err(|_| "fixture closed")?;
        self.inner.resolve()
    }
}
pub(crate) fn held(fixture: &Fixture) -> (Loader, Control) {
    let (ready, started) = bounded(4);
    let (release, finish) = bounded(4);
    let loader = Loader::with_resolver(
        fixture.theme.clone(),
        Held {
            inner: fixture.resolver(),
            started: ready,
            release: finish,
        },
        Duration::from_secs(30),
    )
    .unwrap();
    (loader, Control { started, release })
}
pub(crate) fn until(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < end, "theme result did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}
