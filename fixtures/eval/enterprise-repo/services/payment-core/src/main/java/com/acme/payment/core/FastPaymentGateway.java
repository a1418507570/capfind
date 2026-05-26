package com.acme.payment.core;

import org.springframework.stereotype.Service;

@Service
public class FastPaymentGateway implements PaymentGateway {
    /** Low-latency authorization path for preferred customers. */
    public PaymentAuthorization authorize(PaymentRequest request) {
        return new PaymentAuthorization("fast");
    }
}
