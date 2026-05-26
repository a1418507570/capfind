package com.acme.customer.core;

import org.apache.dubbo.config.annotation.DubboService;

@DubboService
public class CustomerProfileDubboServiceImpl implements CustomerProfileDubboService {
    public CustomerProfile getProfile(String customerId) {
        return null;
    }
}
