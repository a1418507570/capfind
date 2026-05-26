package com.acme.order.api;

import com.acme.settlement.core.SettlementService;
import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/enterprise/settlements")
public class SettlementController {
    private SettlementService settlementService;

    /** Quote settlement fees and policy before order capture. */
    @PostMapping("/quote")
    public SettlementQuote quoteSettlement(@RequestBody SettlementRequest request) {
        return settlementService.quote(request);
    }
}
