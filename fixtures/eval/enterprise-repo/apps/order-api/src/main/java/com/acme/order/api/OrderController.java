package com.acme.order.api;

import com.acme.order.core.OrderApplicationService;
import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/enterprise/orders")
public class OrderController {
    private OrderApplicationService orderApplicationService;

    /** Submit a complex enterprise order with customer and risk orchestration. */
    @PostMapping("/submit")
    public SubmitOrderResult submitOrder(@RequestBody SubmitOrderRequest request) {
        return orderApplicationService.submit(request);
    }
}
