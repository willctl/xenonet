use eidolon_mcp::{Error, Part};
use eidolon_testkit::SCREENSHOT_BASE64;
use serde_json::json;

use crate::support::{env, write, Call, DOWNLOADS, PAGE_TEXT};

/// The uid the snapshot gave the element named `name`.
fn uid_of(snapshot: &str, name: &str) -> String {
    let line = snapshot.lines().find(|l| l.contains(&format!("\"{name}\""))).unwrap_or_else(|| panic!("no {name} in\n{snapshot}"));
    line.split('[').nth(1).unwrap().split(']').next().unwrap().to_owned()
}

fn refused(r: eidolon_mcp::Result<String>) -> String {
    match r {
        Err(Error::Refused(text)) => text,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ---- files: the refusals come first ----

#[tokio::test]
async fn an_upload_outside_the_files_folder_is_refused_and_nothing_is_copied() {
    let e = env().await;
    write(&e.dir.0.join("secret.txt"), "not for upload");
    let uid = uid_of(&e.tools.take_snapshot().await.unwrap(), "Avatar");
    for path in ["../secret.txt".to_owned(), "sub/../../secret.txt".to_owned(), e.dir.0.join("secret.txt").display().to_string(), "/etc/hosts".to_owned()] {
        let result = e.tools.upload_file_by_uid(&uid, &path).await;
        assert!(result.is_err(), "{path} was accepted");
    }
    assert_eq!(e.driver.copies(), 0, "a refused upload reached the cell");
    assert_eq!(e.page.files("#upload"), Vec::<String>::new());
}

#[tokio::test]
async fn an_upload_through_a_symlink_that_leads_out_is_refused() {
    let e = env().await;
    write(&e.dir.0.join("secret.txt"), "not for upload");
    write(&e.dir.0.join("outside/inner.txt"), "not for upload");
    std::os::unix::fs::symlink(e.dir.0.join("secret.txt"), e.files.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(e.dir.0.join("outside"), e.files.join("linked-dir")).unwrap();
    let uid = uid_of(&e.tools.take_snapshot().await.unwrap(), "Avatar");
    let text = refused(e.tools.upload_file_by_uid(&uid, "link.txt").await);
    assert!(text.contains("outside the files folder"), "{text}");
    refused(e.tools.upload_file_by_uid(&uid, "linked-dir/inner.txt").await);
    assert_eq!(e.driver.copies(), 0);
    assert_eq!(e.page.files("#upload"), Vec::<String>::new());
}

#[tokio::test]
async fn a_folder_is_not_uploaded() {
    let e = env().await;
    write(&e.files.join("pics/a.png"), "x");
    let uid = uid_of(&e.tools.take_snapshot().await.unwrap(), "Avatar");
    refused(e.tools.upload_file_by_uid(&uid, "pics").await);
    assert_eq!(e.driver.copies(), 0);
}

#[tokio::test]
async fn an_upload_inside_the_files_folder_is_copied_in_and_chosen_by_its_guest_path() {
    let e = env().await;
    write(&e.files.join("pics/avatar.png"), "png bytes");
    let uid = uid_of(&e.tools.take_snapshot().await.unwrap(), "Avatar");
    // Relative, and with a harmless `..` that stays inside.
    let said = e.tools.upload_file_by_uid(&uid, "pics/../pics/avatar.png").await.unwrap();
    assert!(said.contains("avatar.png"), "{said}");
    let guest = "/home/eidolon/files/uploads/avatar.png";
    assert_eq!(e.driver.calls(), vec![Call::CopyIn { host: e.files.join("pics/avatar.png"), guest: guest.to_owned() }]);
    assert_eq!(std::fs::read_to_string(e.driver.guest(guest)).unwrap(), "png bytes");
    assert_eq!(e.page.files("#upload"), vec![guest.to_owned()]);
}

#[tokio::test]
async fn an_absolute_path_inside_the_files_folder_works_too() {
    let e = env().await;
    write(&e.files.join("doc.pdf"), "pdf");
    let uid = uid_of(&e.tools.take_snapshot().await.unwrap(), "Avatar");
    e.tools.upload_file_by_uid(&uid, &e.files.join("doc.pdf").display().to_string()).await.unwrap();
    assert_eq!(e.page.files("#upload"), vec!["/home/eidolon/files/uploads/doc.pdf".to_owned()]);
}

#[tokio::test]
async fn upload_takes_the_firefox_servers_file_path_name_and_the_older_one() {
    let e = env().await;
    write(&e.files.join("a.txt"), "a");
    let uid = uid_of(&e.tools.take_snapshot().await.unwrap(), "Avatar");
    e.tools.call_tool("upload_file_by_uid", json!({ "uid": uid, "filePath": "a.txt" })).await.unwrap();
    e.tools.call_tool("upload_file_by_uid", json!({ "uid": uid, "host_path": "a.txt" })).await.unwrap();
    assert_eq!(e.driver.copies(), 2);
}

#[tokio::test]
async fn fetch_download_refuses_names_with_a_folder_or_dotdot() {
    let e = env().await;
    write(&e.driver.guest(DOWNLOADS).join("report.csv"), "a,b");
    for name in ["../x", "..", ".", "", "a/b", "sub\\x", "/etc/passwd", "report.csv/../../x"] {
        let r = e.tools.fetch_download(name).await;
        assert!(matches!(r, Err(Error::Refused(_))), "{name:?}: {r:?}");
    }
    assert_eq!(e.driver.copies(), 0, "a refused name reached the cell");
    assert!(!e.files.join("downloads").exists());
}

#[tokio::test]
async fn fetch_download_will_not_write_through_a_downloads_symlink_that_leads_out() {
    let e = env().await;
    write(&e.driver.guest(DOWNLOADS).join("report.csv"), "a,b");
    std::fs::create_dir_all(e.dir.0.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(e.dir.0.join("elsewhere"), e.files.join("downloads")).unwrap();
    refused(e.tools.fetch_download("report.csv").await);
    assert_eq!(e.driver.copies(), 0);
    assert!(std::fs::read_dir(e.dir.0.join("elsewhere")).unwrap().next().is_none());
}

#[tokio::test]
async fn fetch_download_copies_a_finished_download_out_and_keeps_earlier_files() {
    let e = env().await;
    write(&e.driver.guest(DOWNLOADS).join("report.csv"), "a,b");
    let first = e.tools.fetch_download("report.csv").await.unwrap();
    assert!(first.contains("downloads/report.csv"), "{first}");
    assert_eq!(std::fs::read_to_string(e.files.join("downloads/report.csv")).unwrap(), "a,b");
    // A second fetch does not overwrite the first.
    write(&e.driver.guest(DOWNLOADS).join("report.csv"), "changed");
    e.tools.fetch_download("report.csv").await.unwrap();
    assert_eq!(std::fs::read_to_string(e.files.join("downloads/report.csv")).unwrap(), "a,b");
    assert_eq!(std::fs::read_to_string(e.files.join("downloads/report-1.csv")).unwrap(), "changed");
    assert!(e.driver.calls().contains(&Call::CopyOut { guest: format!("{DOWNLOADS}/report.csv"), host: e.files.join("downloads/report.csv").canonicalize().unwrap() }));
}

#[tokio::test]
async fn downloads_are_listed_and_a_partial_one_is_not_fetched() {
    let e = env().await;
    write(&e.driver.guest(DOWNLOADS).join("done.txt"), "12345");
    write(&e.driver.guest(DOWNLOADS).join("big.zip.part"), "1");
    let listing = e.tools.list_downloads().await.unwrap();
    assert!(listing.contains("done.txt 5 bytes\n"), "{listing}");
    assert!(listing.contains("big.zip.part 1 bytes (still downloading)"), "{listing}");
    refused(e.tools.fetch_download("big.zip.part").await);
    assert!(matches!(e.tools.fetch_download("missing.txt").await, Err(Error::NoSuchDownload(_))));
    assert_eq!(e.driver.copies(), 0);
    // The listing is a command with arguments, not a shell string.
    let Call::Exec(argv) = &e.driver.calls()[0] else { panic!("no exec") };
    assert_eq!(argv[0], "find");
    assert!(argv.iter().all(|a| !a.contains(';')));
}

#[tokio::test]
async fn no_downloads_reads_as_such() {
    let e = env().await;
    assert_eq!(e.tools.list_downloads().await.unwrap(), "There are no downloads.");
}

// ---- addresses ----

#[tokio::test]
async fn addresses_that_read_the_browser_itself_or_run_script_are_refused() {
    let e = env().await;
    for url in [
        "file:///home/eidolon/profile/cookies.sqlite",
        "about:logins",
        "about:config",
        "javascript:alert(1)",
        "data:text/html,hi",
        "view-source:https://x.test",
        "chrome://browser/content/browser.xhtml",
    ] {
        refused(e.tools.navigate_page(url).await);
        refused(e.tools.new_page(Some(url)).await);
    }
    assert!(e.proxy.methods.lock().unwrap().is_empty(), "a refused address reached the browser");
}

// ---- pages ----

#[tokio::test]
async fn pages_open_list_select_navigate_and_close() {
    let e = env().await;
    let opened = e.tools.new_page(Some("https://example.test/a")).await.unwrap();
    assert!(opened.contains("ctx-2") && opened.contains("https://example.test/a"), "{opened}");
    assert_eq!(e.page.url(), "https://example.test/a");
    let listing = e.tools.list_pages().await.unwrap();
    assert!(listing.contains("ctx-1 ") && listing.contains("ctx-2 (selected)"), "{listing}");
    e.tools.select_page("ctx-1").await.unwrap();
    assert!(e.tools.list_pages().await.unwrap().contains("ctx-1 (selected)"));
    assert!(matches!(e.tools.select_page("ctx-9").await, Err(Error::NoSuchPage(_))));
    e.tools.navigate_page("https://example.test/b").await.unwrap();
    assert_eq!(e.page.url(), "https://example.test/b");
    e.tools.close_page("ctx-2").await.unwrap();
    assert!(!e.tools.list_pages().await.unwrap().contains("ctx-2"));
    assert!(matches!(e.tools.close_page("ctx-2").await, Err(Error::NoSuchPage(_))));
}

#[tokio::test]
async fn history_failing_in_the_browser_keeps_the_uids() {
    let e = env().await;
    let uid = uid_of(&e.tools.take_snapshot().await.unwrap(), "Go");
    // The fake has no history command; the uids are dropped only on success.
    assert!(e.tools.call_tool("navigate_history", json!({ "direction": "back" })).await.is_err());
    e.tools.click_by_uid(&uid).await.unwrap();
    assert!(matches!(e.tools.call_tool("navigate_history", json!({ "direction": "sideways" })).await, Err(Error::BadArguments(_))));
}

// ---- snapshot and acting ----

#[tokio::test]
async fn a_snapshot_gives_uids_and_click_fill_and_type_reach_the_right_element() {
    let e = env().await;
    let snapshot = e.tools.take_snapshot().await.unwrap();
    assert!(snapshot.starts_with("page: https://example.test/signup (Sign up)\n"), "{snapshot}");
    assert!(snapshot.contains("form:\n  \"Your details\"\n  [e"), "{snapshot}");
    assert!(snapshot.contains("textbox \"Name\" id=\"name\" type=\"text\""), "{snapshot}");

    let (name, go, avatar) = (uid_of(&snapshot, "Name"), uid_of(&snapshot, "Go"), uid_of(&snapshot, "Avatar"));
    assert!(name != go && go != avatar && name != avatar);

    e.tools.click_by_uid(&go).await.unwrap();
    assert_eq!(e.page.clicks_on("#go"), 1);
    assert_eq!(e.page.clicks(), 1, "the click reached another element");

    e.tools.fill_by_uid(&name, "Ada").await.unwrap();
    assert_eq!(e.page.field_value("#name").as_deref(), Some("Ada"));
    assert_eq!(e.page.clicks_on("#go"), 1);

    e.tools.type_text(" L").await.unwrap();
    assert_eq!(e.page.field_value("#name").as_deref(), Some("Ada L"));

    // The select-all that replaces old content is a script, not key presses
    // that would land in the field.
    let scripts = e.proxy.scripts.lock().unwrap().clone();
    assert!(scripts.iter().any(|s| s.contains("eidolon:prepare-fill")));
}

#[tokio::test]
async fn a_snapshot_changes_nothing_in_the_page() {
    let e = env().await;
    let state = |e: &crate::support::Env| (e.page.url(), e.page.title(), e.page.clicks(), e.page.field_value("#name"), e.page.files("#upload"));
    let before = state(&e);
    e.tools.take_snapshot().await.unwrap();
    e.tools.take_snapshot().await.unwrap();
    e.tools.get_page_text().await.unwrap();
    assert_eq!(before, state(&e));

    // Only reads reached the browser: no input, no file choice, no navigation.
    let methods = e.proxy.methods.lock().unwrap().clone();
    assert!(methods.iter().all(|m| m == "browsingContext.getTree" || m == "script.callFunction"), "{methods:?}");

    // And the script has no way to write: nothing that sets an attribute,
    // a class, a style, markup or text, or acts on the page.
    let js = eidolon_mcp::snapshot::SNAPSHOT_JS;
    for forbidden in [
        "setAttribute",
        "removeAttribute",
        "dataset",
        "classList",
        ".className",
        "innerHTML",
        "outerHTML",
        "insertAdjacent",
        "appendChild",
        "insertBefore",
        "removeChild",
        "replaceChild",
        ".style.",
        "textContent =",
        "innerText =",
        "createElement",
        ".remove()",
        "dispatchEvent",
        ".focus(",
        ".click(",
        "fetch(",
        "XMLHttpRequest",
    ] {
        assert!(!js.contains(forbidden), "the snapshot script has {forbidden}");
    }
}

#[tokio::test]
async fn uids_stop_working_after_navigation_and_are_never_reused() {
    let e = env().await;
    let go = uid_of(&e.tools.take_snapshot().await.unwrap(), "Go");
    e.tools.navigate_page("https://example.test/next").await.unwrap();
    assert!(matches!(e.tools.click_by_uid(&go).await, Err(Error::UnknownUid(_))));
    let go_again = uid_of(&e.tools.take_snapshot().await.unwrap(), "Go");
    assert_ne!(go, go_again, "an old uid names a new element");
    assert!(matches!(e.tools.click_by_uid(&go).await, Err(Error::UnknownUid(_))));
    e.tools.click_by_uid(&go_again).await.unwrap();
    assert_eq!(e.page.clicks_on("#go"), 1);
}

#[tokio::test]
async fn an_unknown_uid_says_to_take_a_snapshot() {
    let e = env().await;
    let err = e.tools.click_by_uid("e99").await.unwrap_err();
    assert!(err.to_string().contains("take_snapshot"), "{err}");
}

#[tokio::test]
async fn an_element_the_browser_lost_says_to_take_a_snapshot_again() {
    let e = env().await;
    // The heading is node-9, which the fake page does not have: what a
    // stale element looks like to the browser.
    let heading = uid_of(&e.tools.take_snapshot().await.unwrap(), "Sign up");
    let err = e.tools.click_by_uid(&heading).await.unwrap_err();
    assert!(matches!(err, Error::StaleElement), "{err:?}");
    assert!(err.to_string().contains("take_snapshot"));
}

#[tokio::test]
async fn keys_press_as_names_and_combinations() {
    let e = env().await;
    e.tools.press_key("Control+a").await.unwrap();
    e.tools.press_key("Enter").await.unwrap();
    e.tools.press_key("F5").await.unwrap();
    assert!(matches!(e.tools.press_key("Nonsense").await, Err(Error::BadArguments(_))));
    assert!(matches!(e.tools.press_key("a+Control").await, Err(Error::BadArguments(_))));
}

#[tokio::test]
async fn text_and_screenshot_come_back() {
    let e = env().await;
    assert_eq!(e.tools.get_page_text().await.unwrap(), PAGE_TEXT);
    assert_eq!(e.tools.screenshot_page().await.unwrap(), SCREENSHOT_BASE64);
    let parts = e.tools.call_tool("screenshot_page", json!({})).await.unwrap();
    assert_eq!(parts, vec![Part::Image { data: SCREENSHOT_BASE64.to_owned(), mime: "image/png" }]);
}

#[tokio::test]
async fn bad_arguments_and_unknown_tools_are_errors_not_actions() {
    let e = env().await;
    assert!(matches!(e.tools.call_tool("click_by_uid", json!({})).await, Err(Error::BadArguments(_))));
    assert!(matches!(e.tools.call_tool("click_by_uid", json!({ "uid": "e1", "extra": 1 })).await, Err(Error::BadArguments(_))));
    // There is no tool that runs a script of the agent's own.
    assert!(matches!(e.tools.call_tool("evaluate_script", json!({ "function": "1" })).await, Err(Error::BadArguments(_))));
    assert!(e.proxy.methods.lock().unwrap().is_empty());
}
