package com.acme.payment.core;

public interface PaymentGateway {
    PaymentAuthorization authorize(PaymentRequest request);
}
