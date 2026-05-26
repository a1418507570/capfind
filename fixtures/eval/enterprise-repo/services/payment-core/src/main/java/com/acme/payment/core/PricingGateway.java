package com.acme.payment.core;

import com.acme.pricing.client.PricingPolicyClient;
import com.acme.pricing.client.PricingPolicyRequest;
import org.springframework.stereotype.Service;

@Service
public class PricingGateway {
    private PricingPolicyClient pricingPolicyClient;

    /** Wrap pricing policy client access behind a domain-friendly payment facade. */
    public PricingPolicy quoteAuthorizationFee(PaymentRequest request) {
        return pricingPolicyClient.quotePolicy(
            new PricingPolicyRequest(request.orderId(), request.currency())
        );
    }
}
