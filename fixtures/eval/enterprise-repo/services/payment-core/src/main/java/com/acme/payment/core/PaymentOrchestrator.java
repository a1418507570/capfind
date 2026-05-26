package com.acme.payment.core;

import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.stereotype.Service;

@Service
public class PaymentOrchestrator {
    @Autowired
    private PaymentGateway paymentGateway;
    private PricingGateway pricingGateway;

    /** Coordinate pricing policy and payment authorization for enterprise checkout. */
    public PaymentAuthorization authorizePayment(PaymentRequest request) {
        PricingPolicy policy = pricingGateway.quoteAuthorizationFee(request);
        PaymentAuthorization authorization = paymentGateway.authorize(request);
        return authorization.withPolicy(policy.code());
    }
}
