use assert_cmd::prelude::*;
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

const CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/mdm")
public class MdmController {
    /** Query MDM by criteria. */
    @PostMapping("/query")
    public ApiResult queryMdm(@RequestBody MdmQueryRequest request) {
        return service.query(request);
    }
}
"#;

fn capfind() -> Command {
    Command::cargo_bin("capfind").unwrap()
}

fn fixture_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src/main/java/com/demo");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("MdmController.java"), CONTROLLER).unwrap();
    dir
}

#[test]
fn init_creates_config_and_capfindignore() {
    let dir = tempfile::tempdir().unwrap();

    capfind()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    assert!(dir.path().join(".capfind/config.toml").exists());
    assert!(dir.path().join(".capfindignore").exists());
    assert!(fs::read_to_string(dir.path().join(".gitignore"))
        .unwrap()
        .contains(".capfind/"));
}

#[test]
fn index_find_show_stats_work_end_to_end() {
    let dir = fixture_repo();

    capfind()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    capfind()
        .current_dir(dir.path())
        .arg("index")
        .assert()
        .success();

    let find = capfind()
        .current_dir(dir.path())
        .args(["find", "mdm", "query", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&find).unwrap();
    let first = &json["results"][0];
    assert_eq!(first["kind"], "HttpEndpoint");
    assert_eq!(first["http"]["method"], "POST");
    assert_eq!(first["http"]["path"], "/mdm/query");
    assert!(first["file"]
        .as_str()
        .unwrap()
        .ends_with("MdmController.java"));

    let id = first["id"].as_u64().unwrap().to_string();
    capfind()
        .current_dir(dir.path())
        .args(["show", id.as_str()])
        .assert()
        .success();
    capfind()
        .current_dir(dir.path())
        .arg("stats")
        .assert()
        .success();
}

#[test]
fn explain_and_config_validation_are_wired() {
    let dir = fixture_repo();

    capfind()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    fs::write(
        dir.path().join(".capfind/config.toml"),
        "version = 1\n[search]\nk1 = 1.6\nb = 0.3\n",
    )
    .unwrap();
    capfind()
        .current_dir(dir.path())
        .arg("index")
        .assert()
        .success();

    let explain = capfind()
        .current_dir(dir.path())
        .args(["explain", "mdm", "query", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&explain).unwrap();
    assert!(json["results"][0]["explain"]["bm25_raw"].as_f64().unwrap() > 0.0);

    fs::write(
        dir.path().join(".capfind/config.toml"),
        "version = 1\n[search]\nb = 2.0\n",
    )
    .unwrap();
    capfind()
        .current_dir(dir.path())
        .args(["find", "mdm", "query"])
        .assert()
        .failure();
}
