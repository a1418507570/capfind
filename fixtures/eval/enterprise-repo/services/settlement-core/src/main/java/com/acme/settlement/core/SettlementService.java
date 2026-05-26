package com.acme.settlement.core;

import com.acme.pricing.sdk.Money;
import com.acme.pricing.sdk.PricingClient;
import org.springframework.stereotype.Service;

@Service
public class SettlementService {
    private PricingClient pricingClient;
    private SettlementMapper settlementMapper;

    /** Quote settlement with pricing SDK and settlement policy lookup. */
    public SettlementQuote quote(SettlementRequest request) {
        Money fee = pricingClient.calculateFee(request.orderId(), request.currency());
        SettlementPolicy policy = settlementMapper.selectPolicy(request.region());
        return new SettlementQuote(fee, policy);
    }
}
