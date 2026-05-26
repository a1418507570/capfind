package com.acme.pricing.client;

import org.springframework.cloud.openfeign.FeignClient;
import org.springframework.web.bind.annotation.PostMapping;

@FeignClient(name = "pricing-policy", url = "${pricing.policy.url}")
public interface PricingPolicyClient {
    /** Quote the pricing policy fee used by payment authorization. */
    @PostMapping("/pricing/policy/quote")
    PricingPolicy quotePolicy(PricingPolicyRequest request);
}
