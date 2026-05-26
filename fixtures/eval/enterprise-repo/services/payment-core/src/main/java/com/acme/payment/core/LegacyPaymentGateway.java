package com.acme.payment.core;

import org.springframework.stereotype.Service;

@Service
public class LegacyPaymentGateway implements PaymentGateway {
    /** Legacy authorization path kept for regulated payment rails. */
    public PaymentAuthorization authorize(PaymentRequest request) {
        return new PaymentAuthorization("legacy");
    }
}
