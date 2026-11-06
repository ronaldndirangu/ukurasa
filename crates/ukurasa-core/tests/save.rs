use std::fs;
use std::path::{Path, PathBuf};

use ukurasa_core::{Edit, EditError, SaveError, Stamp, save, save_as};
use ukurasa_edit::lopdf::{Document, Object, dictionary};
use ukurasa_edit::{Family, Span, TextBox, text_lines};

fn workspace(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn source(dir: &Path) -> PathBuf {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages });
    let media_box: Vec<Object> = [0, 0, 612, 792].into_iter().map(Object::from).collect();
    doc.objects.insert(
        pages,
        dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page.into()],
            "Count" => 1,
            "MediaBox" => media_box,
        }
        .into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    let path = dir.join("source.pdf");
    doc.save(&path).unwrap();
    path
}

fn text(page: usize) -> Vec<Edit> {
    let span = Span {
        text: "Approved".into(),
        font: "Helvetica".into(),
        face: None,
        family: Family::Sans,
        bold: false,
        italic: false,
        underline: false,
        size: 14.0,
        color: [0, 0, 0],
    };
    vec![Edit::Text(TextBox {
        page,
        x: 72.0,
        y: 72.0,
        lines: vec![vec![span]],
    })]
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn writes_and_replaces_the_target() {
    let dir = workspace("writes");
    let source = source(&dir);
    let original = fs::read(&source).unwrap();
    let target = dir.join("signed.pdf");
    fs::write(&target, b"old").unwrap();

    save_as(&source, &target, &text(0)).unwrap();

    let saved = fs::read(&target).unwrap();
    assert!(saved.starts_with(&original));
    let doc = Document::load_mem(&saved).unwrap();
    assert_eq!(text_lines(&doc, 0).unwrap()[0].text, "Approved");
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(files(&dir), ["signed.pdf", "source.pdf"]);
}

#[test]
fn refuses_to_overwrite_the_source() {
    let dir = workspace("same");
    let source = source(&dir);
    let original = fs::read(&source).unwrap();
    for target in [source.clone(), dir.join(".").join("source.pdf")] {
        let error = save_as(&source, &target, &text(0)).unwrap_err();
        assert!(matches!(error, SaveError::SameFile), "{error:?}");
    }
    assert_eq!(fs::read(&source).unwrap(), original);
}

#[test]
fn failed_saves_leave_the_target_untouched() {
    let dir = workspace("failed");
    let source = source(&dir);
    let target = dir.join("signed.pdf");
    fs::write(&target, b"old").unwrap();

    let error = save_as(&source, &target, &text(4)).unwrap_err();
    assert!(
        matches!(error, SaveError::Edit(EditError::NoPage(5))),
        "{error:?}"
    );
    assert_eq!(fs::read(&target).unwrap(), b"old");

    let error = save_as(&source, &dir.join("missing").join("x.pdf"), &text(0)).unwrap_err();
    assert!(matches!(error, SaveError::Write { .. }), "{error:?}");
    assert_eq!(files(&dir), ["signed.pdf", "source.pdf"]);
}

#[test]
fn saves_over_the_open_file() {
    let dir = workspace("in-place");
    let source = source(&dir);
    let original = fs::read(&source).unwrap();
    let opened = Stamp::of(&source).unwrap();

    save(&source, opened, &text(0)).unwrap();

    let saved = fs::read(&source).unwrap();
    assert!(saved.starts_with(&original) && saved.len() > original.len());
    let doc = Document::load_mem(&saved).unwrap();
    assert_eq!(text_lines(&doc, 0).unwrap()[0].text, "Approved");
    assert_eq!(files(&dir), ["source.pdf"]);
}

#[test]
fn leaves_changed_and_read_only_files_alone() {
    let dir = workspace("guarded");
    let source = source(&dir);
    let opened = Stamp::of(&source).unwrap();
    let mut changed = fs::read(&source).unwrap();
    changed.extend_from_slice(b"\n% changed elsewhere\n");
    fs::write(&source, &changed).unwrap();

    let error = save(&source, opened, &text(0)).unwrap_err();
    assert!(matches!(error, SaveError::Changed { .. }), "{error:?}");
    assert_eq!(fs::read(&source).unwrap(), changed);

    let opened = Stamp::of(&source).unwrap();
    let mut permissions = fs::metadata(&source).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source, permissions.clone()).unwrap();
    let error = save(&source, opened, &text(0)).unwrap_err();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&source, permissions).unwrap();
    assert!(matches!(error, SaveError::ReadOnly { .. }), "{error:?}");
    assert_eq!(fs::read(&source).unwrap(), changed);

    let error = save(&source, opened, &text(4)).unwrap_err();
    assert!(
        matches!(error, SaveError::Edit(EditError::NoPage(5))),
        "{error:?}"
    );
    assert_eq!(fs::read(&source).unwrap(), changed);
    assert_eq!(files(&dir), ["source.pdf"]);
}
