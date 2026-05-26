package com.acme.order.core;

import com.acme.customer.core.CustomerGateway;
import com.acme.risk.client.RiskClient;
import org.springframework.stereotype.Service;

@Service
public class OrderApplicationService {
    private RiskClient riskClient;
    private CustomerGateway customerGateway;
    private OrderMapper orderMapper;

    /** Orchestrate submit order flow across risk, customer, and persistence assets. */
    public SubmitOrderResult submit(SubmitOrderRequest request) {
        RiskDecision decision = riskClient.assess(request.customerId());
        CustomerProfile profile = customerGateway.loadCustomer(request.customerId());
        return orderMapper.insertOrder(request.orderId(), profile.id(), decision.level());
    }
}
