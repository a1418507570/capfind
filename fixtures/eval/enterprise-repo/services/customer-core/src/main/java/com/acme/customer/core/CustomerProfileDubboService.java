package com.acme.customer.core;

public interface CustomerProfileDubboService {
    CustomerProfile getProfile(String customerId);
}
