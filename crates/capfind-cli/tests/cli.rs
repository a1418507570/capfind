use assert_cmd::prelude::*;
use serde_json::{json, Value};
use std::fs;
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use zip::write::FileOptions;

const CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/mdm")
public class MdmController {
    private MdmService service;

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

const SERVICE: &str = r#"
package com.demo;

import org.springframework.stereotype.Service;

@Service
public class MdmService {
    private MdmRepository repository;

    public ApiResult query(MdmQueryRequest request) {
        return repository.findById(request.id());
    }
}
"#;

const REPOSITORY: &str = r#"
package com.demo;

import org.springframework.stereotype.Repository;

@Repository
public class MdmRepository {
    public MdmEntity findById(Long id) {
        return null;
    }
}
"#;

const INTERFACE_CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/pricing")
public class PricingController {
    private PricingService pricingService;

    @PostMapping("/quote")
    public Price quote(PriceRequest request) {
        return pricingService.quote(request);
    }
}
"#;

const PRICING_SERVICE: &str = r#"
package com.demo;

public interface PricingService {
    Price quote(PriceRequest request);
}
"#;

const PRICING_SERVICE_IMPL: &str = r#"
package com.demo;

import org.springframework.stereotype.Service;

@Service
public class PricingServiceImpl implements PricingService {
    public Price quote(PriceRequest request) {
        return null;
    }
}
"#;

const AUDIT_SERVICE: &str = r#"
package com.demo;

import org.springframework.stereotype.Service;

@Service
public class AuditService {
    public Price quote(PriceRequest request) {
        return null;
    }
}
"#;

const INVENTORY_CLIENT: &str = r#"
package com.demo;

import org.springframework.cloud.openfeign.FeignClient;
import org.springframework.web.bind.annotation.GetMapping;

@FeignClient(name = "inventory", path = "/inventory")
public interface InventoryClient {
    @GetMapping("/items/{id}")
    InventoryItem lookup(String id);
}
"#;

const INVENTORY_SERVICE: &str = r#"
package com.demo;

import org.springframework.stereotype.Service;

@Service
public class InventoryService {
    private InventoryClient inventoryClient;

    public InventoryItem loadInventory(String id) {
        return inventoryClient.lookup(id);
    }
}
"#;

const CATALOG_DUBBO_SERVICE: &str = r#"
package com.demo;

public interface CatalogDubboService {
    CatalogItem getCatalog(String id);
}
"#;

const CATALOG_DUBBO_SERVICE_IMPL: &str = r#"
package com.demo;

import org.apache.dubbo.config.annotation.DubboService;

@DubboService
public class CatalogDubboServiceImpl implements CatalogDubboService {
    public CatalogItem getCatalog(String id) {
        return null;
    }
}
"#;

const CATALOG_GATEWAY: &str = r#"
package com.demo;

import org.apache.dubbo.config.annotation.DubboReference;
import org.springframework.stereotype.Service;

@Service
public class CatalogGateway {
    @DubboReference
    private CatalogDubboService catalogDubboService;

    public CatalogItem loadCatalog(String id) {
        return catalogDubboService.getCatalog(id);
    }
}
"#;

const ORDER_SERVICE: &str = r#"
package com.demo;

import org.springframework.stereotype.Service;

@Service
public class OrderService {
    private OrderMapper orderMapper;

    public Order load(String id) {
        return orderMapper.selectById(id);
    }
}
"#;

const ORDER_MAPPER_XML: &str = r#"
<?xml version="1.0" encoding="UTF-8" ?>
<mapper namespace="com.demo.OrderMapper">
  <select id="selectById" parameterType="java.lang.String" resultType="com.demo.Order">
    select * from orders where id = #{id}
  </select>
</mapper>
"#;

const VENDOR_GATEWAY: &str = r#"
package com.demo;

import com.vendor.ClientApi;
import org.springframework.stereotype.Service;

@Service
public class VendorGateway {
    private ClientApi clientApi;

    public QueryResponse queryVendor(QueryRequest request) {
        return clientApi.query(request, 10);
    }
}
"#;

const CONSTRUCTOR_PRICING_CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/constructor-pricing")
public class ConstructorPricingController {
    private final PricingService pricingService;

    public ConstructorPricingController(PricingService pricingService) {
        this.pricingService = pricingService;
    }

    @PostMapping("/quote")
    public Price quoteWithConstructor(PriceRequest request) {
        return pricingService.quote(request);
    }
}
"#;

const RESOURCE_INVENTORY_GATEWAY: &str = r#"
package com.demo;

import jakarta.annotation.Resource;
import org.springframework.stereotype.Service;

@Service
public class ResourceInventoryGateway {
    @Resource
    private InventoryClient inventoryClient;

    public InventoryItem resourceLookup(String id) {
        return inventoryClient.lookup(id);
    }
}
"#;

const AUTOWIRED_REPOSITORY_GATEWAY: &str = r#"
package com.demo;

import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.stereotype.Service;

@Service
public class AutowiredRepositoryGateway {
    @Autowired
    private MdmRepository repository;

    public MdmEntity autowiredLoad(Long id) {
        return repository.findById(id);
    }
}
"#;

const FRAUD_CHECK_SERVICE: &str = r#"
package com.demo;

public interface FraudCheckService {
    FraudDecision check(FraudRequest request);
}
"#;

const FAST_FRAUD_CHECK_SERVICE: &str = r#"
package com.demo;

import org.springframework.stereotype.Service;

@Service
public class FastFraudCheckService implements FraudCheckService {
    public FraudDecision check(FraudRequest request) {
        return null;
    }
}
"#;

const SLOW_FRAUD_CHECK_SERVICE: &str = r#"
package com.demo;

import org.springframework.stereotype.Service;

@Service
public class SlowFraudCheckService implements FraudCheckService {
    public FraudDecision check(FraudRequest request) {
        return null;
    }
}
"#;

const FRAUD_CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/fraud")
public class FraudController {
    private final FraudCheckService fraudCheckService;

    public FraudController(FraudCheckService fraudCheckService) {
        this.fraudCheckService = fraudCheckService;
    }

    @PostMapping("/check")
    public FraudDecision check(FraudRequest request) {
        return fraudCheckService.check(request);
    }
}
"#;

const LEGAL_PERSON_CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/legal-person")
public class LegalPersonController {
    @GetMapping("/id-card/account-name")
    public LegalPersonAccountNameResponse getLegalPersonIdCardAccountName(LegalPersonIdCardQuery request) {
        return null;
    }
}
"#;

const ID_CARD_CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/idCard")
public class IdCardController {
    @GetMapping("/idCardOcrAndVerify")
    public IdCardVerifyResponse idCardOcrAndVerify(IdCardImageRequest request) {
        return null;
    }

    @GetMapping("/verifyNameAndNo")
    public VerifyResponse verifyNameAndNo(IdCardVerifyRequest request) {
        return null;
    }
}
"#;

const CUSTOMER_SERVICE_WRAPPER: &str = r#"
package com.demo;

import java.util.List;
import org.springframework.stereotype.Service;

@Service
public class CustomerServiceWrapper {
    /** Returns AccountInfo fields such as legalPersonId, accountName, and relatedAccountList. */
    public List<AccountInfo> selectAccountListByLegalPersonId(String legalPersonId) {
        return null;
    }
}
"#;

const ASYNC_CALL_SERVICE: &str = r#"
package com.demo;

import java.util.concurrent.CompletableFuture;
import org.springframework.stereotype.Service;

@Service
public class AsyncCallService {
    /** Loads legalPersonId and accountName from account context. */
    public CompletableFuture<String> getLegalPersonIdAsync(AccountInfo accountInfo) {
        return null;
    }
}
"#;

const CHECK_TOOL_CONTROLLER: &str = r#"
package com.demo;

import java.util.List;
import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/checkTool/v1")
public class CheckToolController {
    /** Returns AccountInfo logs with legalPersonId, accountName, and relatedAccountList. */
    @GetMapping("/getLogs")
    public List<AccountInfo> getLogs(AccountInfo query) {
        return null;
    }
}
"#;

const VENDOR_CLIENT_API_SOURCE: &str = r#"
package com.vendor;

public class ClientApi {
    /** Query vendor data. */
    public QueryResponse query(QueryRequest request, int timeout) {
        return null;
    }
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

fn git(dir: &TempDir, args: &[&str]) {
    let mut command = Command::new("git");
    command
        .current_dir(dir.path())
        .args(args)
        .assert()
        .success();
}

fn node_type<'a>(nodes: &'a [Value], id: &str) -> Option<&'a str> {
    nodes
        .iter()
        .find(|node| node["id"] == id)
        .and_then(|node| node["type"].as_str())
}

fn node_framework<'a>(nodes: &'a [Value], id: &str) -> Option<&'a str> {
    nodes
        .iter()
        .find(|node| node["id"] == id)
        .and_then(|node| node["framework"].as_str())
}

fn source_jar_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in entries {
        archive.start_file(*name, options).unwrap();
        archive.write_all(content.as_bytes()).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

fn fixture_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src/main/java/com/demo");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("MdmController.java"), CONTROLLER).unwrap();
    fs::write(src_dir.join("UsesJackson.java"), JAVA_EXTERNAL_USER).unwrap();
    fs::write(src_dir.join("MdmService.java"), SERVICE).unwrap();
    fs::write(src_dir.join("MdmRepository.java"), REPOSITORY).unwrap();
    fs::write(src_dir.join("PricingController.java"), INTERFACE_CONTROLLER).unwrap();
    fs::write(src_dir.join("PricingService.java"), PRICING_SERVICE).unwrap();
    fs::write(
        src_dir.join("PricingServiceImpl.java"),
        PRICING_SERVICE_IMPL,
    )
    .unwrap();
    fs::write(src_dir.join("AuditService.java"), AUDIT_SERVICE).unwrap();
    fs::write(src_dir.join("InventoryClient.java"), INVENTORY_CLIENT).unwrap();
    fs::write(src_dir.join("InventoryService.java"), INVENTORY_SERVICE).unwrap();
    fs::write(
        src_dir.join("CatalogDubboService.java"),
        CATALOG_DUBBO_SERVICE,
    )
    .unwrap();
    fs::write(
        src_dir.join("CatalogDubboServiceImpl.java"),
        CATALOG_DUBBO_SERVICE_IMPL,
    )
    .unwrap();
    fs::write(src_dir.join("CatalogGateway.java"), CATALOG_GATEWAY).unwrap();
    fs::write(src_dir.join("OrderService.java"), ORDER_SERVICE).unwrap();
    fs::write(src_dir.join("VendorGateway.java"), VENDOR_GATEWAY).unwrap();
    fs::write(
        src_dir.join("ConstructorPricingController.java"),
        CONSTRUCTOR_PRICING_CONTROLLER,
    )
    .unwrap();
    fs::write(
        src_dir.join("ResourceInventoryGateway.java"),
        RESOURCE_INVENTORY_GATEWAY,
    )
    .unwrap();
    fs::write(
        src_dir.join("AutowiredRepositoryGateway.java"),
        AUTOWIRED_REPOSITORY_GATEWAY,
    )
    .unwrap();
    fs::write(src_dir.join("FraudCheckService.java"), FRAUD_CHECK_SERVICE).unwrap();
    fs::write(
        src_dir.join("FastFraudCheckService.java"),
        FAST_FRAUD_CHECK_SERVICE,
    )
    .unwrap();
    fs::write(
        src_dir.join("SlowFraudCheckService.java"),
        SLOW_FRAUD_CHECK_SERVICE,
    )
    .unwrap();
    fs::write(src_dir.join("FraudController.java"), FRAUD_CONTROLLER).unwrap();
    fs::write(dir.path().join("pom.xml"), POM).unwrap();
    let mapper_dir = dir.path().join("src/main/resources/mapper");
    fs::create_dir_all(&mapper_dir).unwrap();
    fs::write(mapper_dir.join("OrderMapper.xml"), ORDER_MAPPER_XML).unwrap();
    let libs_dir = dir.path().join("libs");
    fs::create_dir_all(&libs_dir).unwrap();
    fs::write(
        libs_dir.join("vendor-api-1.0-sources.jar"),
        source_jar_bytes(&[("com/vendor/ClientApi.java", VENDOR_CLIENT_API_SOURCE)]),
    )
    .unwrap();
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
fn init_product_config_generates_safe_agent_templates() {
    let dir = fixture_repo();
    let generated_dir = dir.path().join("target/generated/com/noisy");
    fs::create_dir_all(&generated_dir).unwrap();
    fs::write(
        generated_dir.join("NoisyGenerated.java"),
        "package com.noisy.generated; public class NoisyGenerated {}",
    )
    .unwrap();
    fs::write(dir.path().join(".capfindignore"), "server/**\n").unwrap();

    capfind()
        .current_dir(dir.path())
        .args(["init", "--product-config"])
        .assert()
        .success();

    let suggested_config = dir.path().join(".capfind/config.suggested.toml");
    let suggested = fs::read_to_string(&suggested_config).unwrap();
    assert!(suggested.contains("include = ["));
    assert!(suggested.contains("\"**/*.java\""));
    assert!(!suggested.contains("\"**/*.go\""));
    assert!(suggested.contains("[ownership.modules]"));
    assert!(suggested.contains("[ownership.packages]"));
    assert!(suggested.contains("com.demo"));
    assert!(!suggested.contains("com.noisy"));
    assert!(suggested.contains("[ownership.external]"));
    assert!(suggested.contains("com.fasterxml.jackson.core"));
    assert!(suggested.contains("[services."));
    assert!(suggested.contains("slo = "));

    let generic_mcp =
        fs::read_to_string(dir.path().join(".capfind/integrations/mcp.generic.json")).unwrap();
    let generic_json: Value = serde_json::from_str(&generic_mcp).unwrap();
    assert_eq!(generic_json["mcpServers"]["capfind"]["args"][0], "mcp");
    assert!(dir
        .path()
        .join(".capfind/integrations/agent-rules.md")
        .exists());
    let agent_rules =
        fs::read_to_string(dir.path().join(".capfind/integrations/agent-rules.md")).unwrap();
    assert!(agent_rules.contains("use capfind before text search"));
    assert!(agent_rules.contains("rg/git grep after capfind"));
    assert!(dir
        .path()
        .join(".capfind/integrations/mcp-client.md")
        .exists());
    let mcp_client =
        fs::read_to_string(dir.path().join(".capfind/integrations/mcp-client.md")).unwrap();
    assert!(mcp_client.contains("before rg/git grep"));

    fs::write(&suggested_config, "sentinel").unwrap();
    capfind()
        .current_dir(dir.path())
        .args(["init", "--product-config"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&suggested_config).unwrap(), "sentinel");

    capfind()
        .current_dir(dir.path())
        .args(["init", "--product-config", "--force"])
        .assert()
        .success();
    assert!(fs::read_to_string(&suggested_config)
        .unwrap()
        .contains("Suggested capfind product config"));
}

#[test]
fn index_find_show_stats_work_end_to_end() {
    let dir = fixture_repo();
    fs::write(
        dir.path()
            .join("src/main/java/com/demo/LegalPersonController.java"),
        LEGAL_PERSON_CONTROLLER,
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("src/main/java/com/demo/IdCardController.java"),
        ID_CARD_CONTROLLER,
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("src/main/java/com/demo/CustomerServiceWrapper.java"),
        CUSTOMER_SERVICE_WRAPPER,
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("src/main/java/com/demo/AsyncCallService.java"),
        ASYNC_CALL_SERVICE,
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("src/main/java/com/demo/CheckToolController.java"),
        CHECK_TOOL_CONTROLLER,
    )
    .unwrap();

    capfind()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    fs::write(
        dir.path().join(".capfind/config.toml"),
        r#"
version = 1

[ownership.modules]
"src/main" = "Demo Service Owners"

[ownership.packages]
"com.demo" = "Demo Domain"

[ownership.external]
"com.fasterxml.jackson.core" = "Runtime Libraries"

[services."demo-service"]
name = "Demo Service"
module = "src/main"
package = "com.demo"
owner = "Demo Service Owners"
tier = "gold"
slo = "99.9%"
runbook = "docs/runbooks/demo-service.md"
tags = ["agent-facing"]
"#,
    )
    .unwrap();
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

    let chinese_context = capfind()
        .current_dir(dir.path())
        .args(["context", "获取法人身份证账号姓名", "--limit", "5"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let chinese_context_json: Value = serde_json::from_slice(&chinese_context).unwrap();
    assert!(chinese_context_json["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| {
            candidate["name"] == "GET /legal-person/id-card/account-name"
                && candidate["entrypoints"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entrypoint| {
                        entrypoint["kind"] == "http"
                            && entrypoint["value"] == "GET /legal-person/id-card/account-name"
                    })
                && candidate["evidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|evidence| {
                        evidence["file"]
                            .as_str()
                            .is_some_and(|file| file.ends_with("LegalPersonController.java"))
                    })
        }));

    let chinese_diagnose = capfind()
        .current_dir(dir.path())
        .args(["diagnose-query", "获取法人身份证账号姓名"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let chinese_diagnose_json: Value = serde_json::from_slice(&chinese_diagnose).unwrap();
    assert_eq!(
        chinese_diagnose_json["diagnosis"],
        "ranked_candidates_available"
    );
    assert!(chinese_diagnose_json["phrase_expansions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|expansion| expansion["phrase"] == "法人"));

    let account_agent = capfind()
        .current_dir(dir.path())
        .args([
            "agent",
            "从现有代码里找到可支持的接口",
            "获取法人身份证",
            "账号",
            "姓名",
            "--limit",
            "8",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let account_agent_json: Value = serde_json::from_slice(&account_agent).unwrap();
    let account_candidates = account_agent_json["candidates"].as_array().unwrap();
    let rank = |method: &str| {
        account_candidates
            .iter()
            .position(|candidate| candidate["method"] == method)
    };
    let account_rank = rank("selectAccountListByLegalPersonId").unwrap();
    assert!(rank("getLegalPersonIdAsync").is_some());
    assert!(rank("getLogs").is_some());
    assert!(account_rank < rank("idCardOcrAndVerify").unwrap());
    assert!(account_rank < rank("verifyNameAndNo").unwrap());

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
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == "capfind_context" }));
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| {
            tool["name"] == "capfind_context"
                && tool["input_schema"]["properties"]["record_shown"]["type"] == "boolean"
        }));
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| {
            tool["name"] == "capfind_context"
                && tool["description"]
                    .as_str()
                    .is_some_and(|description| description.contains("before rg/git grep"))
        }));
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == "capfind_diagnose_query" }));
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == "capfind_diagnose_file" }));
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == "capfind_map" }));
    assert!(mcp_tools_json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == "capfind_detect_adoption" }));

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

    let context = capfind()
        .current_dir(dir.path())
        .args(["context", "mdm", "query", "--no-auto-index"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let context_json: Value = serde_json::from_slice(&context).unwrap();
    assert_eq!(context_json["schema_version"], "capfind.context.v1");
    assert!(context_json["has_candidates"].as_bool().unwrap());
    assert!(context_json["candidates"][0]["entrypoints"]
        .as_array()
        .is_some_and(|entrypoints| !entrypoints.is_empty()));

    let mcp_context = capfind()
        .current_dir(dir.path())
        .args([
            "mcp",
            "--call",
            "capfind_context",
            "--args",
            r#"{"task":"jackson object mapper","limit":2,"record_shown":true,"session_id":"agent-run-1"}"#,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_context_json: Value = serde_json::from_slice(&mcp_context).unwrap();
    assert_eq!(mcp_context_json["tool"], "capfind_context");
    assert_eq!(
        mcp_context_json["result"]["schema_version"],
        "capfind.context.v1"
    );
    assert_eq!(
        mcp_context_json["result"]["adoption_recording"]["stage"],
        "shown"
    );
    assert_eq!(
        mcp_context_json["result"]["adoption_recording"]["session_id"],
        "agent-run-1"
    );
    let shown_recorded = mcp_context_json["result"]["adoption_recording"]["recorded"]
        .as_u64()
        .unwrap();
    assert!(shown_recorded > 0);
    assert_eq!(
        shown_recorded as usize,
        mcp_context_json["result"]["candidates"]
            .as_array()
            .unwrap()
            .len()
    );

    let query_diagnosis = capfind()
        .current_dir(dir.path())
        .args(["diagnose-query", "mdm", "query", "--no-auto-index"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let query_diagnosis_json: Value = serde_json::from_slice(&query_diagnosis).unwrap();
    assert_eq!(
        query_diagnosis_json["schema_version"],
        "capfind.query_diagnosis.v1"
    );
    assert_eq!(
        query_diagnosis_json["diagnosis"],
        "ranked_candidates_available"
    );
    assert!(query_diagnosis_json["tokens"]
        .as_array()
        .is_some_and(|tokens| !tokens.is_empty()));
    assert!(query_diagnosis_json["filtered_top"]
        .as_array()
        .is_some_and(|hits| !hits.is_empty()));

    let filtered_out_diagnosis = capfind()
        .current_dir(dir.path())
        .args([
            "diagnose-query",
            "mdm",
            "query",
            "--lang",
            "proto",
            "--kind",
            "endpoint",
            "--no-auto-index",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let filtered_out_json: Value = serde_json::from_slice(&filtered_out_diagnosis).unwrap();
    assert_eq!(filtered_out_json["diagnosis"], "filters_removed_all_hits");
    assert!(
        filtered_out_json["result_counts"]["filter_loss"]
            .as_u64()
            .unwrap()
            > 0
    );

    let mcp_query_diagnosis = capfind()
        .current_dir(dir.path())
        .args([
            "mcp",
            "--call",
            "capfind_diagnose_query",
            "--args",
            r#"{"query":"mdm query","limit":3}"#,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_query_diagnosis_json: Value = serde_json::from_slice(&mcp_query_diagnosis).unwrap();
    assert_eq!(mcp_query_diagnosis_json["tool"], "capfind_diagnose_query");
    assert_eq!(
        mcp_query_diagnosis_json["result"]["schema_version"],
        "capfind.query_diagnosis.v1"
    );

    let file_diagnosis = capfind()
        .current_dir(dir.path())
        .args([
            "diagnose-file",
            "src/main/java/com/demo/MdmController.java",
            "--no-auto-index",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let file_diagnosis_json: Value = serde_json::from_slice(&file_diagnosis).unwrap();
    assert_eq!(
        file_diagnosis_json["schema_version"],
        "capfind.file_diagnosis.v1"
    );
    assert_eq!(
        file_diagnosis_json["diagnosis"],
        "indexed_with_capabilities"
    );
    assert!(
        file_diagnosis_json["index"]["indexed_capabilities"]
            .as_u64()
            .unwrap()
            > 0
    );

    fs::write(
        dir.path().join("src/main/java/com/demo/PlainUtility.java"),
        "package com.demo;\npublic class PlainUtility {}\n",
    )
    .unwrap();
    let zero_file_diagnosis = capfind()
        .current_dir(dir.path())
        .args([
            "diagnose-file",
            "src/main/java/com/demo/PlainUtility.java",
            "--no-auto-index",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let zero_file_json: Value = serde_json::from_slice(&zero_file_diagnosis).unwrap();
    assert_eq!(
        zero_file_json["diagnosis"],
        "parser_produced_zero_capabilities"
    );

    fs::write(dir.path().join(".capfindignore"), "**/Ignored.java\n").unwrap();
    fs::write(
        dir.path().join("src/main/java/com/demo/Ignored.java"),
        CONTROLLER.replace("MdmController", "Ignored"),
    )
    .unwrap();
    let ignored_file_diagnosis = capfind()
        .current_dir(dir.path())
        .args([
            "diagnose-file",
            "src/main/java/com/demo/Ignored.java",
            "--no-auto-index",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ignored_file_json: Value = serde_json::from_slice(&ignored_file_diagnosis).unwrap();
    assert_eq!(ignored_file_json["diagnosis"], "ignored_by_capfindignore");

    let mcp_file_diagnosis = capfind()
        .current_dir(dir.path())
        .args([
            "mcp",
            "--call",
            "capfind_diagnose_file",
            "--args",
            r#"{"file":"src/main/java/com/demo/MdmController.java"}"#,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_file_diagnosis_json: Value = serde_json::from_slice(&mcp_file_diagnosis).unwrap();
    assert_eq!(mcp_file_diagnosis_json["tool"], "capfind_diagnose_file");
    assert_eq!(
        mcp_file_diagnosis_json["result"]["schema_version"],
        "capfind.file_diagnosis.v1"
    );

    let eval_dir = dir.path().join("fixtures/eval");
    fs::create_dir_all(&eval_dir).unwrap();
    fs::write(
        eval_dir.join("queries.jsonl"),
        r#"{"id":"java-http-mdm","query":"mdm query endpoint","limit":5,"expected":[{"kind":"HttpEndpoint","http_method":"POST","http_path":"/mdm/query","method":"queryMdm"}]}
{"id":"go-http-mdm","query":"go mdm query","limit":5,"lang":"go","expected":[{"lang":"Go","http_method":"GET","http_path":"/v1/go/mdm/query"}]}
{"id":"jackson-dependency","query":"jackson databind","limit":5,"expected":[{"is_reference":true,"signature_contains":"jackson-databind"}]}
"#,
    )
    .unwrap();
    fs::write(
        eval_dir.join("files.jsonl"),
        r#"{"id":"controller-indexed","file":"src/main/java/com/demo/MdmController.java","expected_diagnosis":"indexed_with_capabilities","signals":{"indexed_file":true}}
"#,
    )
    .unwrap();
    fs::write(
        eval_dir.join("edges.jsonl"),
        r#"{"id":"controller-to-service","relationship":"calls","source":{"kind":"HttpEndpoint","method":"queryMdm"},"target":{"kind":"ServiceMethod","class":"MdmService","method":"query"},"evidence":{"call":"service.query"}}
{"id":"controller-exposes-service","relationship":"exposes","source":{"kind":"HttpEndpoint","method":"queryMdm"},"target":{"kind":"ServiceMethod","class":"MdmService","method":"query"},"evidence":{"call":"service.query","derived_from":"calls"}}
{"id":"service-wraps-mybatis","relationship":"wraps","source":{"kind":"ServiceMethod","class":"OrderService","method":"load"},"target":{"kind":"DaoMethod","class":"OrderMapper","method":"selectById","tags_contains":["framework:mybatis_xml"]},"evidence":{"call":"orderMapper.selectById","derived_from":"calls_data_mapper"}}
{"id":"service-to-mybatis","relationship":"calls_data_mapper","source":{"kind":"ServiceMethod","class":"OrderService","method":"load"},"target":{"kind":"DaoMethod","class":"OrderMapper","method":"selectById","tags_contains":["framework:mybatis_xml"]},"evidence":{"call":"orderMapper.selectById","target_framework":"mybatis_xml"}}
"#,
    )
    .unwrap();
    fs::write(
        eval_dir.join("diagnostics.jsonl"),
        r#"{"id":"ambiguous-fraud-call","kind":"ambiguous_call_target","reason":"ambiguous_interface_implementations","call":"fraudCheckService.check","candidate_count_min":2,"candidates":[{"owner":"FastFraudCheckService"},{"owner":"SlowFraudCheckService"}]}
{"id":"ambiguous-fraud-dependency","kind":"ambiguous_dependency_target","reason":"ambiguous_interface_implementations","dependency_name":"fraudCheckService","declared_type":"FraudCheckService","candidate_count_min":2}
"#,
    )
    .unwrap();
    fs::write(
        eval_dir.join("ownerships.jsonl"),
        r#"{"id":"src-main-module-owner","node_kind":"module","name":"src/main","owner":"Demo Service Owners","source":"config.modules","ownership_kind":"module_owner"}
{"id":"demo-package-owner","node_kind":"class","name":"MdmController","owner":"Demo Domain","source":"config.packages","ownership_kind":"package_owner"}
{"id":"jackson-dependency-owner","type":"external_dependency","name_contains":"jackson-databind","owner":"Runtime Libraries","source":"config.external","ownership_kind":"external_owner"}
"#,
    )
    .unwrap();
    let eval = capfind()
        .current_dir(dir.path())
        .args([
            "eval",
            "--no-auto-index",
            "--fail-under-recall",
            "1.0",
            "--fail-under-file-pass",
            "1.0",
            "--fail-under-edge-pass",
            "1.0",
            "--fail-under-diagnostic-pass",
            "1.0",
            "--fail-under-ownership-pass",
            "1.0",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let eval_json: Value = serde_json::from_slice(&eval).unwrap();
    assert_eq!(eval_json["schema_version"], "capfind.eval.v1");
    assert_eq!(eval_json["metrics"]["query"]["cases"], 3);
    assert_eq!(eval_json["metrics"]["query"]["recall_at_k"], 1.0);
    assert_eq!(eval_json["metrics"]["file"]["diagnosis_pass_rate"], 1.0);
    assert_eq!(eval_json["metrics"]["edge"]["cases"], 4);
    assert_eq!(eval_json["metrics"]["edge"]["edge_pass_rate"], 1.0);
    assert_eq!(eval_json["metrics"]["diagnostic"]["cases"], 2);
    assert_eq!(
        eval_json["metrics"]["diagnostic"]["diagnostic_pass_rate"],
        1.0
    );
    assert_eq!(eval_json["metrics"]["ownership"]["cases"], 3);
    assert_eq!(
        eval_json["metrics"]["ownership"]["ownership_pass_rate"],
        1.0
    );
    assert!(eval_json["suite"]
        .as_str()
        .is_some_and(|suite| !suite.is_empty()));
    assert_eq!(
        eval_json["history"]["schema_version"],
        "capfind.eval_history.v1"
    );
    assert_eq!(eval_json["history"]["suite_entries"], 1);
    assert_eq!(
        eval_json["history"]["latest"]["metrics"]["query"]["recall_at_k"],
        1.0
    );
    assert!(dir.path().join(".capfind/eval-history.jsonl").exists());
    let eval_no_history = capfind()
        .current_dir(dir.path())
        .args(["eval", "--no-auto-index", "--no-record-history"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let eval_no_history_json: Value = serde_json::from_slice(&eval_no_history).unwrap();
    assert_eq!(eval_no_history_json["history"]["suite_entries"], 1);

    let asset_map = capfind()
        .current_dir(dir.path())
        .args(["map", "--limit", "80", "--no-auto-index"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let asset_map_json: Value = serde_json::from_slice(&asset_map).unwrap();
    assert_eq!(asset_map_json["schema_version"], "capfind.asset_map.v1");
    assert!(asset_map_json["modules"]
        .as_array()
        .is_some_and(|modules| !modules.is_empty()));
    assert!(asset_map_json["graph"]["nodes"]
        .as_array()
        .is_some_and(|nodes| !nodes.is_empty()));
    assert!(asset_map_json["graph"]["edges"]
        .as_array()
        .is_some_and(|edges| !edges.is_empty()));
    let map_nodes = asset_map_json["graph"]["nodes"].as_array().unwrap();
    let map_edges = asset_map_json["graph"]["edges"].as_array().unwrap();
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "calls"
            && edge["evidence"]["call"] == "service.query"
            && node_type(map_nodes, edge["from"].as_str().unwrap()) == Some("http_endpoint")
            && node_type(map_nodes, edge["to"].as_str().unwrap()) == Some("service_method")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "exposes"
            && edge["evidence"]["call"] == "service.query"
            && edge["evidence"]["derived_from"] == "calls"
            && node_type(map_nodes, edge["from"].as_str().unwrap()) == Some("http_endpoint")
            && node_type(map_nodes, edge["to"].as_str().unwrap()) == Some("service_method")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "calls"
            && edge["evidence"]["call"] == "repository.findById"
            && node_type(map_nodes, edge["from"].as_str().unwrap()) == Some("service_method")
            && node_type(map_nodes, edge["to"].as_str().unwrap()) == Some("dao_method")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "calls"
            && edge["confidence"] == "high"
            && edge["evidence"]["call"] == "pricingService.quote"
            && edge["evidence"]["resolved_type"] == "PricingService"
            && edge["evidence"]["resolved_declaring_class"] == "PricingServiceImpl"
            && edge["evidence"]["resolution"] == "implements_interface"
            && node_type(map_nodes, edge["from"].as_str().unwrap()) == Some("http_endpoint")
            && node_type(map_nodes, edge["to"].as_str().unwrap()) == Some("service_method")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "calls_http_client"
            && edge["evidence"]["call"] == "inventoryClient.lookup"
            && edge["evidence"]["target_framework"] == "feign_client"
            && node_framework(map_nodes, edge["to"].as_str().unwrap()) == Some("feign_client")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "wraps"
            && edge["evidence"]["call"] == "inventoryClient.lookup"
            && edge["evidence"]["derived_from"] == "calls_http_client"
            && node_framework(map_nodes, edge["to"].as_str().unwrap()) == Some("feign_client")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "calls_rpc"
            && edge["evidence"]["call"] == "catalogDubboService.getCatalog"
            && edge["evidence"]["resolution"] == "implements_interface"
            && edge["evidence"]["target_framework"] == "dubbo_service"
            && node_type(map_nodes, edge["to"].as_str().unwrap()) == Some("rpc_method")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "calls_data_mapper"
            && edge["evidence"]["call"] == "orderMapper.selectById"
            && edge["evidence"]["target_framework"] == "mybatis_xml"
            && node_type(map_nodes, edge["to"].as_str().unwrap()) == Some("dao_method")
            && node_framework(map_nodes, edge["to"].as_str().unwrap()) == Some("mybatis_xml")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "calls_external_api"
            && edge["evidence"]["call"] == "clientApi.query"
            && edge["evidence"]["target_type"] == "sources_jar_method"
            && node_type(map_nodes, edge["to"].as_str().unwrap()) == Some("sources_jar_method")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "depends_on"
            && edge["to"] == "class:PricingServiceImpl"
            && edge["evidence"]["kind"] == "java_constructor_injection"
            && edge["evidence"]["dependency_name"] == "pricingService"
            && edge["evidence"]["resolution"] == "implements_interface"
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "depends_on"
            && edge["to"] == "class:InventoryClient"
            && edge["evidence"]["kind"] == "java_field_injection"
            && edge["evidence"]["dependency_name"] == "inventoryClient"
            && edge["evidence"]["annotations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|annotation| annotation == "Resource")
    }));
    assert!(map_edges.iter().any(|edge| {
        edge["relationship"] == "depends_on"
            && edge["to"] == "class:MdmRepository"
            && edge["evidence"]["kind"] == "java_field_injection"
            && edge["evidence"]["dependency_name"] == "repository"
            && edge["evidence"]["annotations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|annotation| annotation == "Autowired")
    }));
    let map_diagnostics = asset_map_json["graph"]["diagnostics"].as_array().unwrap();
    assert!(map_diagnostics.iter().any(|diagnostic| {
        diagnostic["kind"] == "ambiguous_call_target"
            && diagnostic["reason"] == "ambiguous_interface_implementations"
            && diagnostic["call"] == "fraudCheckService.check"
            && diagnostic["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| candidate["owner"] == "FastFraudCheckService")
            && diagnostic["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| candidate["owner"] == "SlowFraudCheckService")
    }));
    assert!(map_diagnostics.iter().any(|diagnostic| {
        diagnostic["kind"] == "ambiguous_dependency_target"
            && diagnostic["reason"] == "ambiguous_interface_implementations"
            && diagnostic["dependency_name"] == "fraudCheckService"
    }));

    let mcp_map = capfind()
        .current_dir(dir.path())
        .args(["mcp", "--call", "capfind_map", "--args", r#"{"limit":80}"#])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_map_json: Value = serde_json::from_slice(&mcp_map).unwrap();
    assert_eq!(mcp_map_json["tool"], "capfind_map");
    assert_eq!(
        mcp_map_json["result"]["schema_version"],
        "capfind.asset_map.v1"
    );
    let src_main_module = mcp_map_json["result"]["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["name"] == "src/main")
        .unwrap();
    assert_eq!(src_main_module["ownership"]["owner"], "Demo Service Owners");
    assert_eq!(src_main_module["ownership"]["source"], "config.modules");
    assert_eq!(src_main_module["service"]["name"], "Demo Service");
    assert_eq!(src_main_module["service"]["slo"], "99.9%");
    assert!(mcp_map_json["result"]["services"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|service| {
            service["id"] == "demo-service"
                && service["capability_count"]
                    .as_u64()
                    .is_some_and(|count| count > 0)
        }));
    assert!(mcp_map_json["result"]["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| {
            node["kind"] == "capability"
                && node["package"] == "com.demo"
                && node["ownership"]["owner"] == "Demo Domain"
                && node["ownership"]["source"] == "config.packages"
        }));
    assert!(mcp_map_json["result"]["external"]["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|dependency| {
            dependency["ownership"]["owner"] == "Runtime Libraries"
                && dependency["ownership"]["source"] == "config.external"
        }));

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
    let mcp_show_args = format!(
        r#"{{"id":{},"task":"add mdm query endpoint","record_inspected":true,"session_id":"agent-run-1"}}"#,
        id
    );
    let mcp_show_inspected = capfind()
        .current_dir(dir.path())
        .args([
            "mcp",
            "--call",
            "capfind_show",
            "--args",
            mcp_show_args.as_str(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_show_inspected_json: Value = serde_json::from_slice(&mcp_show_inspected).unwrap();
    assert_eq!(
        mcp_show_inspected_json["result"]["adoption_recording"]["stage"],
        "inspected"
    );
    assert_eq!(
        mcp_show_inspected_json["result"]["adoption_recording"]["session_id"],
        "agent-run-1"
    );
    capfind()
        .current_dir(dir.path())
        .arg("stats")
        .assert()
        .success();

    let doctor = capfind()
        .current_dir(dir.path())
        .args(["doctor", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor_json: Value = serde_json::from_slice(&doctor).unwrap();
    assert_eq!(doctor_json["schema_version"], "capfind.dashboard.v1");
    assert_eq!(doctor_json["index"]["present"], true);

    capfind()
        .current_dir(dir.path())
        .args([
            "record-adoption",
            id.as_str(),
            "--task",
            "add mdm query endpoint",
            "--stage",
            "shown",
            "--session-id",
            "agent-run-1",
        ])
        .assert()
        .success();

    let adoption = capfind()
        .current_dir(dir.path())
        .args([
            "record-adoption",
            id.as_str(),
            "--task",
            "add mdm query endpoint",
            "--file",
            "src/main/java/com/demo/MdmController.java",
            "--session-id",
            "agent-run-1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let adoption_json: Value = serde_json::from_slice(&adoption).unwrap();
    assert_eq!(adoption_json["recorded"], true);
    assert_eq!(adoption_json["session_id"], "agent-run-1");
    assert_eq!(adoption_json["adoption"]["adopted"], 1);
    assert_eq!(adoption_json["adoption"]["funnel"]["events"]["adopted"], 1);

    let rejection = capfind()
        .current_dir(dir.path())
        .args([
            "record-adoption",
            id.as_str(),
            "--task",
            "add mdm query endpoint",
            "--stage",
            "rejected",
            "--rejected-reason",
            "wrong_ownership_boundary",
            "--note",
            "must call external owner directly",
            "--session-id",
            "agent-run-2",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rejection_json: Value = serde_json::from_slice(&rejection).unwrap();
    assert_eq!(rejection_json["recorded"], true);
    assert_eq!(rejection_json["stage"], "rejected");
    assert_eq!(
        rejection_json["rejected_reason"],
        "wrong_ownership_boundary"
    );
    assert_eq!(rejection_json["adoption"]["rejected"], 1);

    let dashboard = capfind()
        .current_dir(dir.path())
        .args(["dashboard", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let dashboard_json: Value = serde_json::from_slice(&dashboard).unwrap();
    assert_eq!(dashboard_json["adoption"]["final_events"], 2);
    assert_eq!(dashboard_json["adoption"]["adoption_rate"], 0.5);
    assert_eq!(
        dashboard_json["adoption"]["funnel"]["events"]["shown"],
        shown_recorded + 1
    );
    assert_eq!(
        dashboard_json["adoption"]["funnel"]["events"]["inspected"],
        1
    );
    assert_eq!(
        dashboard_json["adoption"]["rejection_taxonomy"]["by_reason"]["wrong_ownership_boundary"],
        1
    );
    let candidate_rollup = dashboard_json["adoption"]["funnel_by_candidate"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["candidate_id"].as_u64().unwrap() == id.parse::<u64>().unwrap())
        .unwrap();
    assert!(candidate_rollup["stage_path"]
        .as_array()
        .unwrap()
        .iter()
        .any(|stage| stage == "shown"));
    assert!(candidate_rollup["stage_path"]
        .as_array()
        .unwrap()
        .iter()
        .any(|stage| stage == "adopted"));
    assert!(candidate_rollup["session_ids"]
        .as_array()
        .unwrap()
        .iter()
        .any(|session| session == "agent-run-1"));
    let task_rollup = dashboard_json["adoption"]["funnel_by_task"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["task"] == "add mdm query endpoint")
        .unwrap();
    assert!(task_rollup["candidate_ids"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate_id| candidate_id.as_u64().unwrap() == id.parse::<u64>().unwrap()));
    let session_rollup = dashboard_json["adoption"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["session_id"] == "agent-run-1")
        .unwrap();
    assert_eq!(session_rollup["outcome"], "adopted");
    assert!(session_rollup["stage_path"]
        .as_array()
        .unwrap()
        .iter()
        .any(|stage| stage == "inspected"));
    let quality = &dashboard_json["adoption"]["quality"];
    assert_eq!(quality["schema_version"], "capfind.adoption_quality.v1");
    assert!(quality["summary"]["candidate_count"]
        .as_u64()
        .is_some_and(|count| count > 0));
    let candidate_quality = quality["candidate_quality"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["candidate_id"].as_u64().unwrap() == id.parse::<u64>().unwrap())
        .unwrap();
    assert!(candidate_quality["adoption_probability"]
        .as_f64()
        .is_some_and(|probability| probability > 0.0));
    assert!(candidate_quality["confidence"]
        .as_f64()
        .is_some_and(|confidence| confidence > 0.0));
    assert!(candidate_quality["recommended_action"]
        .as_str()
        .is_some_and(|action| !action.is_empty()));
    assert!(quality["session_quality"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["session_id"] == "agent-run-1"
            && item["adoption_probability"]
                .as_f64()
                .is_some_and(|probability| probability > 0.0)));
    let quality_context = capfind()
        .current_dir(dir.path())
        .args(["context", "add", "mdm", "query", "endpoint", "--limit", "5"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let quality_context_json: Value = serde_json::from_slice(&quality_context).unwrap();
    assert_eq!(
        quality_context_json["recommendation_policy"]["ranking"],
        "bm25_then_adoption_quality"
    );
    assert!(quality_context_json["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |candidate| candidate["id"].as_u64().unwrap() == id.parse::<u64>().unwrap()
                && candidate["quality_signal"]["schema_version"]
                    == "capfind.candidate_quality_signal.v1"
        ));
    assert!(!dashboard_json["architecture"]["module_health"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(dashboard_json["architecture"]["edge_counts"].is_array());
    assert!(dashboard_json["architecture"]["diagnostics"]["total"]
        .as_u64()
        .is_some_and(|total| total > 0));
    assert!(dashboard_json["topology"]["module_links"]
        .as_array()
        .is_some_and(|links| !links.is_empty()));
    assert!(dashboard_json["topology"]["module_drilldowns"]
        .as_array()
        .is_some_and(|modules| !modules.is_empty()));
    assert!(dashboard_json["topology"]["sample_edges"]
        .as_array()
        .is_some_and(|edges| !edges.is_empty()));
    assert!(dashboard_json["topology"]["graph_view"]["nodes"]
        .as_array()
        .is_some_and(|nodes| !nodes.is_empty()));
    assert!(dashboard_json["topology"]["graph_view"]["edges"]
        .as_array()
        .is_some_and(|edges| !edges.is_empty()));
    assert!(dashboard_json["topology"]["ownership_drilldowns"]
        .as_array()
        .is_some_and(|owners| !owners.is_empty()));
    assert!(dashboard_json["topology"]["ownership_drilldowns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|owner| owner["owner"] == "Demo Service Owners"
            && owner["owner_source"] == "config.modules"
            && owner["owner_match"] == "src/main"
            && owner["service"]["name"] == "Demo Service"));
    assert!(dashboard_json["asset_map"]["services"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|service| service["id"] == "demo-service" && service["slo"] == "99.9%"));
    assert_eq!(
        dashboard_json["queries"]["schema_version"],
        "capfind.query_history.v1"
    );
    assert!(dashboard_json["queries"]["events"]
        .as_u64()
        .is_some_and(|events| events > 0));
    assert!(dashboard_json["queries"]["unique_queries"]
        .as_u64()
        .is_some_and(|queries| queries > 0));
    assert!(dashboard_json["queries"]["zero_result_events"]
        .as_u64()
        .is_some_and(|events| events > 0));
    assert!(dashboard_json["queries"]["by_tool"]["find"]
        .as_u64()
        .is_some_and(|events| events > 0));
    assert!(dashboard_json["queries"]["by_tool"]["capfind_context"]
        .as_u64()
        .is_some_and(|events| events > 0));
    assert!(dashboard_json["queries"]["top_queries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["query"] == "mdm query"));
    assert!(dashboard_json["queries"]["zero_result_queries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["query"] == "mdm query"));
    assert!(dashboard_json["queries"]["timeline"]
        .as_array()
        .is_some_and(|timeline| !timeline.is_empty()));
    assert_eq!(
        dashboard_json["evals"]["schema_version"],
        "capfind.eval_history.v1"
    );
    assert!(dashboard_json["evals"]["entries"]
        .as_u64()
        .is_some_and(|entries| entries > 0));
    assert_eq!(
        dashboard_json["evals"]["latest"]["metrics"]["query"]["recall_at_k"],
        1.0
    );
    assert_eq!(dashboard_json["history"]["entries"], 1);
    assert_eq!(
        dashboard_json["history"]["recent"][0]["schema_version"],
        "capfind.dashboard_history.v1"
    );
    assert_eq!(
        dashboard_json["history"]["recent"][0]["queries"]["events"],
        dashboard_json["queries"]["events"]
    );
    assert!(dir.path().join(".capfind/dashboard-history.jsonl").exists());
    assert!(dir.path().join(".capfind/query-history.jsonl").exists());
    let dashboard_no_history = capfind()
        .current_dir(dir.path())
        .args(["dashboard", "--json", "--no-record-history"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let dashboard_no_history_json: Value = serde_json::from_slice(&dashboard_no_history).unwrap();
    assert_eq!(dashboard_no_history_json["history"]["entries"], 1);
    let filtered_dashboard = capfind()
        .current_dir(dir.path())
        .args([
            "dashboard",
            "--json",
            "--no-record-history",
            "--module",
            "src/main",
            "--relationship",
            "calls",
            "--graph-limit",
            "5",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let filtered_dashboard_json: Value = serde_json::from_slice(&filtered_dashboard).unwrap();
    assert_eq!(
        filtered_dashboard_json["view"]["filters"]["module"],
        "src/main"
    );
    assert_eq!(
        filtered_dashboard_json["topology"]["graph_view"]["filters"]["relationships"][0],
        "calls"
    );
    assert!(filtered_dashboard_json["topology"]["graph_view"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .all(|edge| edge["relationship"] == "calls"));
    assert!(filtered_dashboard_json["topology"]["graph_view"]["edges"]
        .as_array()
        .is_some_and(|edges| edges.len() <= 5));
    capfind()
        .current_dir(dir.path())
        .args(["dashboard", "--no-record-history"])
        .assert()
        .success();
    let dashboard_html = fs::read_to_string(dir.path().join(".capfind/dashboard.html")).unwrap();
    assert!(dashboard_html.contains("Service Topology"));
    assert!(dashboard_html.contains("Service Graph"));
    assert!(dashboard_html.contains("Graph Filters"));
    assert!(dashboard_html.contains("Ownership Drilldowns"));
    assert!(dashboard_html.contains("<svg"));
    assert!(dashboard_html.contains("Service Map Diagnostics"));
    assert!(dashboard_html.contains("Adoption Correlation"));
    assert!(dashboard_html.contains("Adoption Quality"));
    assert!(dashboard_html.contains("Query Trends"));
    assert!(dashboard_html.contains("Eval Quality History"));
    assert_eq!(
        dashboard_json["asset_map"]["schema_version"],
        "capfind.asset_map.v1"
    );
}

#[test]
fn detect_adoption_finds_matches_from_git_diff() {
    let dir = fixture_repo();

    git(&dir, &["init"]);
    git(&dir, &["config", "user.name", "capfind"]);
    git(&dir, &["config", "user.email", "capfind@example.com"]);

    capfind()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    git(
        &dir,
        &[
            "add",
            "pom.xml",
            "src",
            "server",
            "proto",
            ".gitignore",
            ".capfindignore",
        ],
    );
    git(&dir, &["commit", "-m", "init"]);

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
    let find_json: Value = serde_json::from_slice(&find).unwrap();
    let endpoint = find_json["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["kind"] == "HttpEndpoint" && result["http"]["path"] == "/mdm/query")
        .unwrap();
    let candidate_id = endpoint["id"].as_u64().unwrap();
    let candidate_id_arg = candidate_id.to_string();

    fs::write(
        dir.path().join("src/main/java/com/demo/UsesJackson.java"),
        r#"
package com.demo.external;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.demo.MdmController;

public class UsesJackson {
    private ObjectMapper mapper;
    private MdmController controller;

    public ApiResult reuse(MdmQueryRequest request) {
        return controller.queryMdm(request);
    }
}
"#,
    )
    .unwrap();

    let detect = capfind()
        .current_dir(dir.path())
        .args([
            "detect-adoption",
            "--since",
            "HEAD",
            "--task",
            "reuse mdm query",
            "--candidate-id",
            &candidate_id_arg,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let detect_json: Value = serde_json::from_slice(&detect).unwrap();
    assert_eq!(
        detect_json["schema_version"],
        "capfind.adoption_detection.v1"
    );
    assert_eq!(detect_json["recorded_events"], 1);
    assert!(detect_json["detections"]
        .as_array()
        .is_some_and(|detections| !detections.is_empty()));
    assert_eq!(detect_json["detections"][0]["candidate_id"], candidate_id);
    assert_eq!(detect_json["detections"][0]["recorded"], true);
    assert_eq!(detect_json["adoption"]["auto_detected"], 1);

    let log = fs::read_to_string(dir.path().join(".capfind/adoptions.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 1);
    assert!(log.contains("\"source\":\"git_diff\""));
    assert!(log.contains("\"detection_key\""));

    let second_detect = capfind()
        .current_dir(dir.path())
        .args([
            "detect-adoption",
            "--since",
            "HEAD",
            "--task",
            "reuse mdm query",
            "--candidate-id",
            &candidate_id_arg,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second_json: Value = serde_json::from_slice(&second_detect).unwrap();
    assert_eq!(second_json["recorded_events"], 0);
    assert_eq!(second_json["skipped_duplicates"], 1);
    assert_eq!(
        fs::read_to_string(dir.path().join(".capfind/adoptions.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
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
fn mcp_stdio_can_auto_index_when_server_runs_first() {
    let dir = fixture_repo();

    let response = run_mcp_stdio(
        dir.path(),
        &["--auto-index"],
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "capfind_search",
                "arguments": {"query": "mdm query", "limit": 1}
            }
        }),
    );

    assert_eq!(response["result"]["isError"], false);
    assert!(response["result"]["structuredContent"]["results"]
        .as_array()
        .is_some_and(|results| !results.is_empty()));
    assert!(dir.path().join(".capfind/index.cfi").exists());
}

#[test]
fn mcp_stdio_handles_initialize_list_and_tool_call_sequence() {
    let dir = fixture_repo();

    let responses = run_mcp_stdio_requests(
        dir.path(),
        &[],
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "capfind-test-client", "version": "0.0.0"}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {}
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "capfind_context",
                    "arguments": {"task": "mdm query endpoint", "limit": 1}
                }
            }),
        ],
    );

    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "capfind");
    assert_eq!(responses[1]["id"], 2);
    assert!(responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "capfind_context"));
    assert_eq!(responses[2]["id"], 3);
    assert_eq!(responses[2]["result"]["isError"], false);
    assert!(responses[2]["result"]["structuredContent"]["candidates"]
        .as_array()
        .is_some_and(|candidates| !candidates.is_empty()));
    assert!(dir.path().join(".capfind/index.cfi").exists());
}

#[test]
fn mcp_stdio_refreshes_stale_index_before_serving() {
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

    std::thread::sleep(Duration::from_millis(20));
    let controller_path = dir.path().join("src/main/java/com/demo/MdmController.java");
    fs::write(
        controller_path,
        CONTROLLER.replace(
            "    }\n}\n",
            "    }\n\n    @GetMapping(\"/refresh\")\n    public ApiResult refreshMdm() { return null; }\n}\n",
        ),
    )
    .unwrap();

    let response = run_mcp_stdio(
        dir.path(),
        &["--auto-index"],
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "capfind_context",
                "arguments": {"task": "refresh mdm endpoint", "limit": 5}
            }
        }),
    );

    assert_eq!(response["result"]["isError"], false);
    let candidates = response["result"]["structuredContent"]["candidates"]
        .as_array()
        .unwrap();
    assert!(candidates.iter().any(|candidate| {
        candidate["entrypoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entrypoint| entrypoint["value"] == "GET /mdm/refresh")
    }));
}

#[test]
fn mcp_stdio_refreshes_when_new_source_file_is_added() {
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

    std::thread::sleep(Duration::from_millis(20));
    let new_controller_path = dir
        .path()
        .join("src/main/java/com/demo/SettlementController.java");
    fs::write(
        new_controller_path,
        r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/settlement")
public class SettlementController {
    @GetMapping("/preview")
    public ApiResult previewSettlement() {
        return null;
    }
}
"#,
    )
    .unwrap();

    let response = run_mcp_stdio(
        dir.path(),
        &["--auto-index"],
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "capfind_context",
                "arguments": {"task": "settlement preview endpoint", "limit": 5}
            }
        }),
    );

    assert_eq!(response["result"]["isError"], false);
    let candidates = response["result"]["structuredContent"]["candidates"]
        .as_array()
        .unwrap();
    assert!(candidates.iter().any(|candidate| {
        candidate["entrypoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entrypoint| entrypoint["value"] == "GET /settlement/preview")
    }));
}

#[test]
fn mcp_stdio_missing_index_returns_helpful_error() {
    let dir = fixture_repo();

    let response = run_mcp_stdio(
        dir.path(),
        &["--no-auto-index"],
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "capfind_search",
                "arguments": {"query": "mdm query", "limit": 1}
            }
        }),
    );

    assert_eq!(response["result"]["isError"], true);
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("capfind index"));
    assert!(text.contains("--auto-index"));
}

fn run_mcp_stdio(cwd: &std::path::Path, extra_args: &[&str], request: Value) -> Value {
    let mut responses = run_mcp_stdio_requests(cwd, extra_args, &[request]);
    assert_eq!(responses.len(), 1);
    responses.remove(0)
}

fn run_mcp_stdio_requests(
    cwd: &std::path::Path,
    extra_args: &[&str],
    requests: &[Value],
) -> Vec<Value> {
    let mut cmd = capfind();
    cmd.current_dir(cwd)
        .arg("mcp")
        .arg("--stdio")
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for request in requests {
            writeln!(stdin, "{}", request).unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
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
