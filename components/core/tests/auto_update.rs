use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_contract::app_internal::section::RemainingTime;
use fvm_core::auto_update::{
    Asset, Check, DOWNLOADED_PERCENT, DownloadProgress, InvalidRelease, Platform, Release, Update,
    Version, check, check_due, failed_install_version, next_check_at, page_check_due,
};

const CURRENT: Version = Version {
    major: 26,
    minor: 1,
    patch: 0,
};

const ALL_FILES: [&str; 4] = [
    "FortiVPNManager-26.2.0.msi",
    "FortiVPNManager-26.2.0.pkg",
    "SHA256SUMS.txt",
    "SHA256SUMS.txt.sig",
];

const NOTES: &str = "\
<!-- lang:en -->
## What's new in 26.2.0

### Automatic updates
- The app checks for a new version once a day and installs it with your approval.
- The download is verified against the signed checksum list.

### Faster reconnects with openconnect
- Reconnects reuse the session when the gateway allows it.

#### Under the hood
- Dependencies are up to date.

### Fixes
- The menu bar icon no longer stays gray after waking from sleep.

<!-- lang:ko -->
## 26.2.0 새 기능

### 자동 업데이트
- 하루 한 번 새 버전을 확인하고, 승인하면 설치합니다.
- 내려받은 파일을 서명된 체크섬 목록으로 확인합니다.

### openconnect 재연결 속도 개선
- 게이트웨이가 허용하면 기존 세션으로 다시 연결합니다.

#### 내부 변경
- 의존성을 최신 버전으로 갱신했습니다.

### 버그 수정
- 잠자기에서 깨어난 뒤 메뉴 막대 아이콘이 회색으로 남던 문제를 고쳤습니다.

**Full Changelog**: https://github.com/qoor/FortiVPNManager/compare/v26.1.0...v26.2.0
";

const LATEST: &str = r#"{
  "url": "https://api.github.com/repos/qoor/FortiVPNManager/releases/261504822",
  "assets_url": "https://api.github.com/repos/qoor/FortiVPNManager/releases/261504822/assets",
  "html_url": "https://github.com/qoor/FortiVPNManager/releases/tag/v26.2.0",
  "id": 261504822,
  "author": {"login": "github-actions[bot]", "id": 41898282, "type": "Bot"},
  "node_id": "RE_kwDOQ3x1Z84PlkY2",
  "tag_name": "v26.2.0",
  "target_commitish": "main",
  "name": "v26.2.0",
  "draft": false,
  "immutable": false,
  "prerelease": false,
  "created_at": "2026-11-16T03:12:45Z",
  "updated_at": "2026-11-16T05:40:02Z",
  "published_at": "2026-11-16T03:20:11Z",
  "assets": [
    {
      "url": "https://api.github.com/repos/qoor/FortiVPNManager/releases/assets/318840001",
      "id": 318840001,
      "name": "FortiVPNManager-26.2.0.msi",
      "label": "",
      "content_type": "application/octet-stream",
      "state": "uploaded",
      "size": 48431104,
      "digest": "sha256:9f2c4e1a6b0d8c7e5f3a1b9d7c5e3f1a2b4c6d8e0f1a3b5c7d9e1f3a5b7c9d1e",
      "download_count": 3,
      "browser_download_url": "https://github.com/qoor/FortiVPNManager/releases/download/v26.2.0/FortiVPNManager-26.2.0.msi"
    },
    {
      "url": "https://api.github.com/repos/qoor/FortiVPNManager/releases/assets/318840002",
      "id": 318840002,
      "name": "FortiVPNManager-26.2.0.pkg",
      "label": "",
      "content_type": "application/octet-stream",
      "state": "uploaded",
      "size": 21733376,
      "digest": null,
      "download_count": 5,
      "browser_download_url": "https://github.com/qoor/FortiVPNManager/releases/download/v26.2.0/FortiVPNManager-26.2.0.pkg"
    },
    {
      "url": "https://api.github.com/repos/qoor/FortiVPNManager/releases/assets/318840003",
      "id": 318840003,
      "name": "SHA256SUMS.txt",
      "label": "",
      "content_type": "text/plain",
      "state": "uploaded",
      "size": 186,
      "download_count": 8,
      "browser_download_url": "https://github.com/qoor/FortiVPNManager/releases/download/v26.2.0/SHA256SUMS.txt"
    },
    {
      "url": "https://api.github.com/repos/qoor/FortiVPNManager/releases/assets/318840004",
      "id": 318840004,
      "name": "SHA256SUMS.txt.sig",
      "label": "",
      "content_type": "application/octet-stream",
      "state": "uploaded",
      "size": 64,
      "download_count": 8,
      "browser_download_url": "https://github.com/qoor/FortiVPNManager/releases/download/v26.2.0/SHA256SUMS.txt.sig"
    }
  ],
  "tarball_url": "https://api.github.com/repos/qoor/FortiVPNManager/tarball/v26.2.0",
  "zipball_url": "https://api.github.com/repos/qoor/FortiVPNManager/zipball/v26.2.0",
  "body": "<!-- lang:en -->\r\n## What's new in 26.2.0\r\n\r\n### Automatic updates\r\n- The app checks for a new version once a day.\r\n\r\n### Fixes\r\n- The menu bar icon no longer stays gray after waking from sleep.\r\n\r\n<!-- lang:ko -->\r\n## 26.2.0 새 기능\r\n\r\n### 자동 업데이트\r\n- 하루 한 번 새 버전을 확인합니다.\r\n\r\n### 버그 수정\r\n- 잠자기에서 깨어난 뒤 메뉴 막대 아이콘이 회색으로 남던 문제를 고쳤습니다.\r\n",
  "mentions_count": 0
}"#;

fn version(text: &str) -> Version {
    Version::parse(text).unwrap_or_else(|| panic!("{text:?} is not a version"))
}

const SIZE: u64 = 21_733_376;

fn release(version_text: &str, body: &str, asset_names: &[&str]) -> Release {
    Release {
        version: version(version_text),
        body: body.to_owned(),
        assets: asset_names
            .iter()
            .map(|name| Asset {
                name: (*name).to_owned(),
                size: SIZE,
            })
            .collect(),
    }
}

fn reply(tag_name: &str, body: Option<&str>, asset_names: &[&str]) -> Vec<u8> {
    let assets: Vec<_> = asset_names
        .iter()
        .map(|name| {
            serde_json::json!({
                "id": 318840001,
                "name": name,
                "label": "",
                "content_type": "application/octet-stream",
                "state": "uploaded",
                "size": SIZE,
                "browser_download_url": format!("https://github.com/qoor/FortiVPNManager/releases/download/{tag_name}/{name}"),
            })
        })
        .collect();
    serde_json::json!({
        "html_url": format!("https://github.com/qoor/FortiVPNManager/releases/tag/{tag_name}"),
        "id": 261504822,
        "tag_name": tag_name,
        "target_commitish": "main",
        "name": tag_name,
        "draft": false,
        "prerelease": false,
        "published_at": "2026-11-16T03:20:11Z",
        "assets": assets,
        "body": body,
    })
    .to_string()
    .into_bytes()
}

fn available(release: &Release, platform: Platform) -> Update {
    match check(CURRENT, release, platform) {
        Check::Available(update) => update,
        other => panic!("{release:?} is not available: {other:?}"),
    }
}

fn titles(body: &str) -> BTreeMap<String, Vec<String>> {
    available(&release("26.2.0", body, &ALL_FILES), Platform::MacOs).release_note_titles
}

fn expected(languages: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    languages
        .iter()
        .map(|(code, titles)| {
            (
                (*code).to_owned(),
                titles.iter().map(|title| (*title).to_owned()).collect(),
            )
        })
        .collect()
}

#[test]
fn a_github_reply() {
    let release = Release::from_json(LATEST.as_bytes()).unwrap();
    assert_eq!(release.version, version("26.2.0"));
    assert_eq!(
        release
            .assets
            .iter()
            .map(|asset| (asset.name.as_str(), asset.size))
            .collect::<Vec<_>>(),
        [
            ("FortiVPNManager-26.2.0.msi", 48_431_104),
            ("FortiVPNManager-26.2.0.pkg", 21_733_376),
            ("SHA256SUMS.txt", 186),
            ("SHA256SUMS.txt.sig", 64),
        ]
    );
    assert_eq!(
        available(&release, Platform::Windows).installer_size,
        48_431_104
    );
    assert!(
        release
            .body
            .starts_with("<!-- lang:en -->\r\n## What's new")
    );
    assert_eq!(
        available(&release, Platform::MacOs).release_note_titles,
        expected(&[
            ("en", &["Automatic updates", "Fixes"]),
            ("ko", &["자동 업데이트", "버그 수정"]),
        ])
    );
}

#[test]
fn a_null_or_missing_body_is_empty() {
    let release = Release::from_json(&reply("v26.2.0", None, &ALL_FILES)).unwrap();
    assert_eq!(release.body, "");
    assert!(
        available(&release, Platform::Windows)
            .release_note_titles
            .is_empty()
    );
    let json = br#"{"tag_name": "v26.2.0", "assets": []}"#;
    assert_eq!(Release::from_json(json).unwrap().body, "");
}

#[test]
fn unreadable_replies() {
    for json in [
        "",
        "not json",
        "null",
        "[]",
        "{}",
        r#"{"tag_name": "v26.2.0", "body": "", "assets": ["#,
        r#"{"body": "", "assets": []}"#,
        r#"{"tag_name": null, "body": "", "assets": []}"#,
        r#"{"tag_name": 26, "body": "", "assets": []}"#,
        r#"{"tag_name": "v26.2.0", "body": ""}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": null}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": {}}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": [{"id": 1}]}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": [{"name": null, "size": 1}]}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": [{"name": "SHA256SUMS.txt"}]}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": [{"name": "SHA256SUMS.txt", "size": null}]}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": [{"name": "SHA256SUMS.txt", "size": -1}]}"#,
        r#"{"tag_name": "v26.2.0", "body": "", "assets": [{"name": "SHA256SUMS.txt", "size": "186"}]}"#,
        r#"{"tag_name": "v26.2.0", "body": 5, "assets": []}"#,
    ] {
        assert_eq!(
            Release::from_json(json.as_bytes()),
            Err(InvalidRelease),
            "{json}"
        );
    }
}

#[test]
fn replies_with_an_invalid_tag() {
    for tag in ["26.2.0", "V26.2.0", "v26.2", "v26.2.0-beta", "test-17", ""] {
        assert_eq!(
            Release::from_json(&reply(tag, Some(NOTES), &ALL_FILES)),
            Err(InvalidRelease),
            "{tag:?}"
        );
    }
}

#[test]
fn an_older_or_equal_release_is_up_to_date() {
    for version_text in ["26.1.0", "26.0.9", "25.12.3", "0.0.0"] {
        for platform in [Platform::MacOs, Platform::Windows] {
            assert_eq!(
                check(CURRENT, &release(version_text, NOTES, &[]), platform),
                Check::UpToDate,
                "{version_text}"
            );
        }
    }
    assert_eq!(
        check(
            version("26.10.0"),
            &release("26.9.0", NOTES, &[]),
            Platform::MacOs
        ),
        Check::UpToDate
    );
}

#[test]
fn a_newer_release_with_every_file_is_available() {
    let latest = Release::from_json(&reply("v26.2.0", Some(NOTES), &ALL_FILES)).unwrap();
    let base = "https://github.com/qoor/FortiVPNManager/releases";
    let notes = expected(&[
        (
            "en",
            &[
                "Automatic updates",
                "Faster reconnects with openconnect",
                "Fixes",
            ],
        ),
        (
            "ko",
            &["자동 업데이트", "openconnect 재연결 속도 개선", "버그 수정"],
        ),
    ]);
    for (platform, file) in [
        (Platform::MacOs, "FortiVPNManager-26.2.0.pkg"),
        (Platform::Windows, "FortiVPNManager-26.2.0.msi"),
    ] {
        assert_eq!(
            check(CURRENT, &latest, platform),
            Check::Available(Update {
                version: version("26.2.0"),
                installer_file_name: file.to_owned(),
                installer_size: SIZE,
                release_page_url: format!("{base}/tag/v26.2.0"),
                installer_url: format!("{base}/download/v26.2.0/{file}"),
                checksums_url: format!("{base}/download/v26.2.0/SHA256SUMS.txt"),
                checksums_signature_url: format!("{base}/download/v26.2.0/SHA256SUMS.txt.sig"),
                release_note_titles: notes.clone(),
            })
        );
    }
    let update = available(
        &release(
            "26.1.1",
            "",
            &[
                "FortiVPNManager-26.1.1.pkg",
                "SHA256SUMS.txt",
                "SHA256SUMS.txt.sig",
            ],
        ),
        Platform::MacOs,
    );
    assert_eq!(update.version, version("26.1.1"));
    assert_eq!(
        update.installer_url,
        format!("{base}/download/v26.1.1/FortiVPNManager-26.1.1.pkg")
    );
}

#[test]
fn a_newer_release_missing_a_file_fails() {
    for (platform, asset_names) in [
        (
            Platform::MacOs,
            &[
                "FortiVPNManager-26.2.0.msi",
                "SHA256SUMS.txt",
                "SHA256SUMS.txt.sig",
            ][..],
        ),
        (
            Platform::MacOs,
            &["FortiVPNManager-26.2.0.pkg", "SHA256SUMS.txt.sig"],
        ),
        (
            Platform::MacOs,
            &["FortiVPNManager-26.2.0.pkg", "SHA256SUMS.txt"],
        ),
        (
            Platform::Windows,
            &[
                "FortiVPNManager-26.2.0.pkg",
                "SHA256SUMS.txt",
                "SHA256SUMS.txt.sig",
            ],
        ),
        (
            Platform::Windows,
            &["FortiVPNManager-26.2.0.msi", "SHA256SUMS.txt.sig"],
        ),
        (
            Platform::Windows,
            &["FortiVPNManager-26.2.0.msi", "SHA256SUMS.txt"],
        ),
        (
            Platform::MacOs,
            &[
                "FortiVPNManager-26.1.0.pkg",
                "SHA256SUMS.txt",
                "SHA256SUMS.txt.sig",
            ],
        ),
        (
            Platform::MacOs,
            &[
                "fortivpnmanager-26.2.0.pkg",
                "SHA256SUMS.txt",
                "SHA256SUMS.txt.sig",
            ],
        ),
        (
            Platform::MacOs,
            &[
                "FortiVPNManager-26.2.0.pkg",
                "sha256sums.txt",
                "SHA256SUMS.txt.sig",
            ],
        ),
        (
            Platform::MacOs,
            &[
                "FortiVPNManager-26.2.0.pkg",
                "SHA256SUMS.txt",
                "SHA256SUMS.sig",
            ],
        ),
        (Platform::MacOs, &[]),
    ] {
        assert_eq!(
            check(CURRENT, &release("26.2.0", NOTES, asset_names), platform),
            Check::Failed,
            "{platform:?} {asset_names:?}"
        );
    }
}

#[test]
fn urls_are_built_not_read_from_the_reply() {
    let json = serde_json::json!({
        "html_url": "https://mirror.example.com/releases/v26.2.0",
        "tag_name": "v26.2.0",
        "body": NOTES,
        "assets": ALL_FILES.iter().map(|name| serde_json::json!({
            "name": name,
            "size": SIZE,
            "browser_download_url": format!("https://mirror.example.com/download/{name}"),
        })).collect::<Vec<_>>(),
    });
    let release = Release::from_json(json.to_string().as_bytes()).unwrap();
    let update = available(&release, Platform::Windows);
    let base = "https://github.com/qoor/FortiVPNManager/releases";
    assert_eq!(update.release_page_url, format!("{base}/tag/v26.2.0"));
    assert_eq!(
        update.installer_url,
        format!("{base}/download/v26.2.0/FortiVPNManager-26.2.0.msi")
    );
    assert_eq!(
        update.checksums_url,
        format!("{base}/download/v26.2.0/SHA256SUMS.txt")
    );
    assert_eq!(
        update.checksums_signature_url,
        format!("{base}/download/v26.2.0/SHA256SUMS.txt.sig")
    );
}

#[test]
fn release_note_titles_per_language() {
    assert_eq!(
        titles(NOTES),
        expected(&[
            (
                "en",
                &[
                    "Automatic updates",
                    "Faster reconnects with openconnect",
                    "Fixes",
                ],
            ),
            (
                "ko",
                &["자동 업데이트", "openconnect 재연결 속도 개선", "버그 수정"],
            ),
        ])
    );
}

#[test]
fn crlf_line_endings() {
    assert_eq!(titles(&NOTES.replace('\n', "\r\n")), titles(NOTES));
    assert_eq!(
        titles("<!-- lang:en -->\r\n### Automatic updates  \r\n### Fixes\r\n"),
        expected(&[("en", &["Automatic updates", "Fixes"])])
    );
}

#[test]
fn only_three_hashes_and_a_space_start_a_title() {
    let body = "\
<!-- lang:en -->
###Missing space
 ### Indented
#### Four
##### Five
## Two
# One
- ### In a bullet
###\tTab
### **Bold** `code` [link](https://example.com)\x20\x20
###    Spaced out\x20\t\x20
";
    assert_eq!(
        titles(body),
        expected(&[(
            "en",
            &["**Bold** `code` [link](https://example.com)", "Spaced out"]
        )])
    );
}

#[test]
fn text_before_the_first_marker_is_ignored() {
    let body = "\
### Highlights
Thanks to everyone who reported issues.
<!-- lang:en -->
### Automatic updates
";
    assert_eq!(titles(body), expected(&[("en", &["Automatic updates"])]));
}

#[test]
fn a_repeated_language_is_appended() {
    let body = "\
<!-- lang:en -->
### Automatic updates
<!-- lang:ko -->
### 자동 업데이트
<!-- lang:en -->
### Fixes
<!-- lang:ko -->
### 버그 수정
";
    assert_eq!(
        titles(body),
        expected(&[
            ("en", &["Automatic updates", "Fixes"]),
            ("ko", &["자동 업데이트", "버그 수정"]),
        ])
    );
}

#[test]
fn a_body_without_markers_has_no_titles() {
    let body = NOTES
        .lines()
        .filter(|line| !line.starts_with("<!--"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(titles(&body).is_empty());
    assert!(titles("").is_empty());
}

#[test]
fn a_title_line_without_text_is_skipped() {
    let body =
        "<!-- lang:en -->\n###\x20\x20\x20\n### \t\n### Fixes\n<!-- lang:ko -->\n###\x20\x20\r\n";
    assert_eq!(titles(body), expected(&[("en", &["Fixes"])]));
}

#[test]
fn a_language_without_titles_is_left_out() {
    let body = "\
<!-- lang:en -->
### Automatic updates
<!-- lang:ko -->
## 26.2.0 새 기능
- 하루 한 번 새 버전을 확인합니다.
";
    assert_eq!(titles(body), expected(&[("en", &["Automatic updates"])]));
}

#[test]
fn marker_lines_may_have_surrounding_spaces() {
    let body =
        "<!-- lang:en -->\n### Automatic updates\n  \t<!-- lang:ko -->   \n### 자동 업데이트\n";
    assert_eq!(
        titles(body),
        expected(&[("en", &["Automatic updates"]), ("ko", &["자동 업데이트"]),])
    );
}

#[test]
fn only_exact_marker_lines_switch_the_language() {
    let mut body = String::from("<!-- lang:en -->\n### Kept in English\n");
    for line in [
        "<!--lang:ko-->",
        "<!--  lang:ko  -->",
        "<!-- lang: ko -->",
        "<!-- lang:KO -->",
        "<!-- lang:ko-kr -->",
        "<!-- lang:ko_KR -->",
        "<!-- lang:ko1 -->",
        "<!-- lang: -->",
        "<!-- lang:ko --> 한국어",
        "<!-- language:ko -->",
        "<!-- LANG:ko -->",
        "- <!-- lang:ko -->",
    ] {
        body.push_str(line);
        body.push_str("\n### ");
        body.push_str(line);
        body.push('\n');
    }
    let titles = titles(&body);
    assert_eq!(titles.keys().collect::<Vec<_>>(), ["en"]);
    assert_eq!(titles["en"].len(), 13);
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn last_check() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_900_000_000)
}

#[test]
fn a_check_is_due_once_a_day_has_passed() {
    let last = last_check();
    for (elapsed, due) in [
        (Duration::ZERO, false),
        (Duration::from_secs(1), false),
        (DAY / 2, false),
        (DAY - Duration::from_secs(1), false),
        (DAY, true),
        (DAY + Duration::from_secs(1), true),
    ] {
        assert_eq!(check_due(last, last + elapsed), due, "{elapsed:?}");
    }
}

#[test]
fn a_check_is_due_at_once_after_a_long_sleep() {
    let last = last_check();
    for slept in [DAY + DAY / 2, DAY * 3, DAY * 30, DAY * 400] {
        assert!(check_due(last, last + slept), "{slept:?}");
    }
}

#[test]
fn a_check_is_due_when_the_clock_was_set_back_before_the_last_check() {
    let last = last_check();
    for set_back in [
        Duration::from_secs(1),
        Duration::from_secs(60 * 60),
        DAY,
        DAY * 365,
    ] {
        assert!(check_due(last, last - set_back), "{set_back:?}");
    }
}

#[test]
fn the_next_check_is_a_day_after_the_last() {
    let last = last_check();
    let next = next_check_at(last);
    assert_eq!(next, last + DAY);
    assert!(!check_due(last, next - Duration::from_secs(1)));
    assert!(check_due(last, next));
    let later = last + DAY * 3 + Duration::from_secs(17);
    assert_eq!(next_check_at(later), later + DAY);
}

#[test]
fn an_install_failed_when_the_app_is_older_than_its_target() {
    for (target, running) in [
        ("26.2.0", "26.1.0"),
        ("26.1.1", "26.1.0"),
        ("26.10.0", "26.9.0"),
        ("27.0.0", "26.10.3"),
    ] {
        assert_eq!(
            failed_install_version(version(target), version(running)),
            Some(version(target)),
            "{target} on {running}"
        );
    }
}

#[test]
fn an_install_did_not_fail_when_the_app_reached_or_passed_its_target() {
    for (target, running) in [
        ("26.2.0", "26.2.0"),
        ("26.2.0", "26.2.1"),
        ("26.2.0", "26.3.0"),
        ("26.9.0", "26.10.0"),
        ("26.2.0", "27.0.0"),
    ] {
        assert_eq!(
            failed_install_version(version(target), version(running)),
            None,
            "{target} on {running}"
        );
    }
}

const MINUTE: Duration = Duration::from_secs(60);

#[test]
fn opening_the_page_checks_once_five_minutes_have_passed() {
    let last = last_check();
    for (elapsed, due) in [
        (Duration::ZERO, false),
        (Duration::from_secs(1), false),
        (MINUTE * 5 - Duration::from_millis(1), false),
        (MINUTE * 5, true),
        (MINUTE * 6, true),
        (DAY, true),
    ] {
        assert_eq!(page_check_due(last, last + elapsed), due, "{elapsed:?}");
    }
    assert!(page_check_due(last, last - Duration::from_secs(1)));
}

fn seconds(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn millis(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn steady(total: u64, per_second: u64, until: u64) -> DownloadProgress {
    let mut progress = DownloadProgress::new(total);
    for second in 1..=until {
        progress.record(seconds(second), per_second * second);
    }
    progress
}

#[test]
fn the_bar_shows_the_downloaded_share_of_sixty() {
    assert_eq!(DOWNLOADED_PERCENT, 60);
    for (received, percent) in [
        (0, 0),
        (1, 0),
        (16, 0),
        (17, 1),
        (500, 30),
        (999, 59),
        (1_000, 60),
        (1_500, 60),
    ] {
        let mut progress = DownloadProgress::new(1_000);
        progress.record(seconds(1), received);
        assert_eq!(progress.percent(), percent, "{received}");
        assert_eq!(progress.received(), received);
        assert_eq!(progress.total(), 1_000);
    }
    let mut big = DownloadProgress::new(u64::MAX);
    big.record(seconds(1), u64::MAX / 2);
    assert_eq!(big.percent(), 29);
    assert_eq!(DownloadProgress::new(0).percent(), 0);
}

#[test]
fn the_remaining_time_is_calculating_for_two_seconds() {
    let mut progress = DownloadProgress::new(1_000_000);
    assert_eq!(progress.remaining(), RemainingTime::Calculating {});
    progress.record(seconds(1), 100_000);
    assert_eq!(progress.remaining(), RemainingTime::Calculating {});
    progress.record(millis(1_999), 199_900);
    assert_eq!(progress.remaining(), RemainingTime::Calculating {});
    progress.record(seconds(2), 200_000);
    assert_eq!(progress.remaining(), RemainingTime::Seconds { seconds: 10 });
}

#[test]
fn the_remaining_time_is_calculating_while_nothing_arrives() {
    let mut progress = DownloadProgress::new(1_000);
    for second in 1..=8 {
        progress.record(seconds(second), 0);
        assert_eq!(progress.remaining(), RemainingTime::Calculating {});
    }
    let mut stalled = steady(1_000, 100, 3);
    for second in 4..=8 {
        stalled.record(seconds(second), 300);
    }
    assert_eq!(stalled.remaining(), RemainingTime::Calculating {});
    stalled.record(seconds(9), 400);
    assert_eq!(stalled.remaining(), RemainingTime::Seconds { seconds: 30 });
}

#[test]
fn over_55_seconds_is_whole_minutes_rounded_up() {
    for (left_seconds, minutes) in [
        (56, 1),
        (59, 1),
        (60, 1),
        (61, 2),
        (119, 2),
        (120, 2),
        (121, 3),
        (3_600, 60),
    ] {
        let progress = steady(10 * (5 + left_seconds), 10, 5);
        assert_eq!(
            progress.remaining(),
            RemainingTime::Minutes { minutes },
            "{left_seconds}"
        );
    }
}

#[test]
fn up_to_55_seconds_is_seconds_rounded_up_to_fives() {
    for (left_tenths, seconds) in [
        (0, 5),
        (1, 5),
        (50, 5),
        (51, 10),
        (100, 10),
        (101, 15),
        (549, 55),
        (550, 55),
    ] {
        let progress = steady(1_000 + 20 * left_tenths, 200, 5);
        assert_eq!(
            progress.remaining(),
            RemainingTime::Seconds { seconds },
            "{left_tenths}"
        );
    }
    for left_tenths in [551, 599, 600] {
        let progress = steady(1_000 + 20 * left_tenths, 200, 5);
        assert_eq!(
            progress.remaining(),
            RemainingTime::Minutes { minutes: 1 },
            "{left_tenths}"
        );
    }
    let done = steady(1_000, 200, 5);
    assert_eq!(done.remaining(), RemainingTime::Seconds { seconds: 5 });
    let mut over = steady(1_000, 200, 5);
    over.record(seconds(6), 1_200);
    assert_eq!(over.remaining(), RemainingTime::Seconds { seconds: 5 });
}

#[test]
fn the_rate_is_the_average_over_the_last_five_seconds() {
    let mut progress = DownloadProgress::new(100_000);
    for second in 1..=3 {
        progress.record(seconds(second), 10_000 * second);
    }
    assert_eq!(progress.remaining(), RemainingTime::Seconds { seconds: 10 });
    for second in 4..=8 {
        progress.record(seconds(second), 30_000 + 1_000 * (second - 3));
    }
    assert_eq!(progress.remaining(), RemainingTime::Minutes { minutes: 2 });
}

#[test]
fn a_late_sample_keeps_at_least_five_seconds_of_rate() {
    let mut progress = DownloadProgress::new(650_000);
    for (at, received) in [
        (1_000, 0),
        (2_001, 0),
        (3_003, 0),
        (4_004, 0),
        (5_006, 0),
        (6_008, 50_000),
    ] {
        progress.record(millis(at), received);
    }
    assert_eq!(progress.remaining(), RemainingTime::Minutes { minutes: 2 });
}
