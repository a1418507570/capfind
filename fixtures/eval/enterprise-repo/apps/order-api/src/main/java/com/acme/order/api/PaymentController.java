package com.acme.order.api;

import com.acme.payment.core.PaymentOrchestrator;
import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/enterprise/payments")
public class PaymentController {
    private PaymentOrchestrator paymentOrchestrator;

    /** Authorize an enterprise payment through pricing and gateway orchestration. */
    @PostMapping("/authorize")
    public PaymentAuthorization authorizePayment(@RequestBody PaymentRequest request) {
        return paymentOrchestrator.authorizePayment(request);
    }
}
