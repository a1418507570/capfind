package com.acme.customer.core;

import org.apache.dubbo.config.annotation.DubboReference;
import org.springframework.stereotype.Service;

@Service
public class CustomerGateway {
    @DubboReference
    private CustomerProfileDubboService customerProfileDubboService;

    public CustomerProfile loadCustomer(String customerId) {
        return customerProfileDubboService.getProfile(customerId);
    }
}
