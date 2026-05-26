package com.acme.risk.client;

import org.springframework.cloud.openfeign.FeignClient;
import org.springframework.web.bind.annotation.PostMapping;

@FeignClient(name = "risk-service", path = "/risk")
public interface RiskClient {
    @PostMapping("/assess")
    RiskDecision assess(String customerId);
}
