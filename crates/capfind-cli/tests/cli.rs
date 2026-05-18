use assert_cmd::prelude::*;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
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

const GO_ROUTES: &str = r#"
package api

func register(router *gin.Engine, chiRouter chi.Router) {
    v1 := router.Group("/v1")
    v1.GET("/go/mdm/query", queryMdm)
    chiRouter.Get("/chi/mdm/query", getMdm)
}
"#;

const PROTO_RPC: &str = r#"
syntax = "proto3";
package demo.mdm.v1;

service MdmService {
  rpc QueryMdm(QueryMdmRequest) returns (QueryMdmResponse) {
    option (google.api.http) = {
      post: "/v1/mdm/query"
      body: "*"
    };
  }
}
"#;

const JAVA_EXTERNAL_USER: &str = r#"
package com.demo.external;

import com.fasterxml.jackson.databind.ObjectMapper;

public class UsesJackson {
    private ObjectMapper mapper;
}
"#;

const POM: &str = r#"
<project>
  <dependencies>
    <dependency>
      <groupId>com.fasterxml.jackson.core</groupId>
      <artifactId>jackson-databind</artifactId>
      <version>2.17.0</version>
    </dependency>
  </dependencies>
</project>
"#;

fn capfind() -> Command {
    Command::cargo_bin("capfind").unwrap()
}

fn fixture_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src/main/java/com/demo");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("MdmController.java"), CONTROLLER).unwrap();
    fs::write(src_dir.join("UsesJackson.java"), JAVA_EXTERNAL_USER).unwrap();
    fs::write(dir.path().join("pom.xml"), POM).unwrap();
    let go_dir = dir.path().join("server/api");
    fs::create_dir_all(&go_dir).unwrap();
    fs::write(go_dir.join("routes.go"), GO_ROUTES).unwrap();
    let proto_dir = dir.path().join("proto/mdm/v1");
    fs::create_dir_all(&proto_dir).unwrap();
    fs::write(proto_dir.join("mdm.proto"), PROTO_RPC).unwrap();
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
    let second_index = capfind()
        .current_dir(dir.path())
        .arg("index")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second_index_stdout = String::from_utf8(second_index).unwrap();
    assert!(second_index_stdout.contains("reused"));
    assert!(second_index_stdout.contains("Parsed 0 files"));

    let find = capfind()
        .current_dir(dir.path())
        .args(["find", "mdm", "query", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&find).unwrap();
    let endpoint = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["kind"] == "HttpEndpoint" && result["http"]["path"] == "/mdm/query")
        .unwrap();
    assert_eq!(endpoint["http"]["method"], "POST");
    assert!(endpoint["file"]
        .as_str()
        .unwrap()
        .ends_with("MdmController.java"));

    let go_find = capfind()
        .current_dir(dir.path())
        .args(["find", "go", "mdm", "query", "--lang", "go", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let go_json: Value = serde_json::from_slice(&go_find).unwrap();
    let go_first = &go_json["results"][0];
    assert_eq!(go_first["lang"], "Go");
    assert_eq!(go_first["http"]["method"], "GET");
    assert_eq!(go_first["http"]["path"], "/v1/go/mdm/query");
    assert!(go_first["file"].as_str().unwrap().ends_with("routes.go"));

    let proto_find = capfind()
        .current_dir(dir.path())
        .args([
            "find", "query", "mdm", "rpc", "--lang", "proto", "--kind", "rpc", "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let proto_json: Value = serde_json::from_slice(&proto_find).unwrap();
    let proto_first = &proto_json["results"][0];
    assert_eq!(proto_first["lang"], "Proto");
    assert_eq!(proto_first["kind"], "RpcMethod");
    assert_eq!(proto_first["method"], "QueryMdm");
    assert_eq!(proto_first["rpc"]["service"], "MdmService");
    assert_eq!(proto_first["rpc"]["rpc"], "QueryMdm");
    assert_eq!(proto_first["rpc"]["req"], "QueryMdmRequest");
    assert_eq!(proto_first["rpc"]["rsp"], "QueryMdmResponse");
    assert_eq!(proto_first["http"]["method"], "POST");
    assert_eq!(proto_first["http"]["path"], "/v1/mdm/query");
    assert!(proto_first["file"].as_str().unwrap().ends_with("mdm.proto"));

    let external_find = capfind()
        .current_dir(dir.path())
        .args(["find", "jackson", "databind", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let external_json: Value = serde_json::from_slice(&external_find).unwrap();
    let external_results = external_json["results"].as_array().unwrap();
    let maven_ref = external_results
        .iter()
        .find(|result| {
            result["signature"]
                == "maven dependency com.fasterxml.jackson.core:jackson-databind:2.17.0"
        })
        .unwrap();
    assert_eq!(maven_ref["is_reference"], true);
    assert!(maven_ref["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tag| tag == "external"));
    assert!(maven_ref["doc"]
        .as_str()
        .unwrap()
        .contains("External jar dependency"));
    assert!(external_results.iter().any(|result| {
        result["signature"] == "import com.fasterxml.jackson.databind.ObjectMapper"
    }));

    let reference_find = capfind()
        .current_dir(dir.path())
        .args(["find", "external", "dependency", "jackson", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let reference_json: Value = serde_json::from_slice(&reference_find).unwrap();
    assert!(reference_json["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|result| {
            result["signature"]
                == "maven dependency com.fasterxml.jackson.core:jackson-databind:2.17.0"
        }));

    let mcp_tools = capfind()
        .current_dir(dir.path())
        .args(["mcp", "--list-tools"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_tools_json: Value = serde_json::from_slice(&mcp_tools).unwrap();
    assert_eq!(mcp_tools_json["schema_version"], "capfind.mcp.v1");
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == "capfind_agent_preflight" }));

    let mcp_search = capfind()
        .current_dir(dir.path())
        .args([
            "mcp",
            "--call",
            "capfind_search",
            "--args",
            r#"{"query":"mdm query","limit":2}"#,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_search_json: Value = serde_json::from_slice(&mcp_search).unwrap();
    assert_eq!(mcp_search_json["tool"], "capfind_search");
    assert!(!mcp_search_json["result"]["results"]
        .as_array()
        .unwrap()
        .is_empty());

    let mut mcp_stdio_child = capfind()
        .current_dir(dir.path())
        .args(["mcp", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    mcp_stdio_child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
"#,
        )
        .unwrap();
    let mcp_stdio_output = mcp_stdio_child.wait_with_output().unwrap();
    assert!(mcp_stdio_output.status.success());
    let mcp_stdio = mcp_stdio_output.stdout;
    let mcp_stdio_json: Value = serde_json::from_slice(&mcp_stdio).unwrap();
    assert_eq!(mcp_stdio_json["jsonrpc"], "2.0");
    assert_eq!(mcp_stdio_json["id"], 1);
    assert!(mcp_stdio_json["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == "capfind_search" }));

    let agent = capfind()
        .current_dir(dir.path())
        .args(["agent", "add", "mdm", "query", "endpoint", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let agent_json: Value = serde_json::from_slice(&agent).unwrap();
    assert_eq!(agent_json["schema_version"], "capfind.agent.v1");
    assert_eq!(agent_json["has_candidates"], true);
    assert_eq!(
        agent_json["recommendation"],
        "review_existing_capability_before_implementing"
    );
    assert_eq!(agent_json["exit_policy"]["fail_on_candidates"], false);
    assert_eq!(agent_json["exit_policy"]["candidate_exit_code"], 0);
    assert_eq!(agent_json["next_actions"][0], "open_candidate_file_lines");
    assert!(!agent_json["candidates"].as_array().unwrap().is_empty());

    let strict_agent = capfind()
        .current_dir(dir.path())
        .args([
            "agent",
            "add",
            "mdm",
            "query",
            "endpoint",
            "--json",
            "--fail-on-candidates",
        ])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let strict_json: Value = serde_json::from_slice(&strict_agent).unwrap();
    assert_eq!(strict_json["exit_policy"]["fail_on_candidates"], true);
    assert_eq!(strict_json["exit_policy"]["candidate_exit_code"], 2);

    let id = endpoint["id"].as_u64().unwrap().to_string();
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
fn agent_can_auto_index_when_hook_runs_first() {
    let dir = fixture_repo();

    let agent = capfind()
        .current_dir(dir.path())
        .args([
            "agent",
            "add",
            "mdm",
            "query",
            "endpoint",
            "--auto-index",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let agent_json: Value = serde_json::from_slice(&agent).unwrap();
    assert_eq!(agent_json["schema_version"], "capfind.agent.v1");
    assert_eq!(agent_json["has_candidates"], true);
    assert_eq!(agent_json["exit_policy"]["fail_on_candidates"], false);
    assert!(dir.path().join(".capfind/index.cfi").exists());
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
