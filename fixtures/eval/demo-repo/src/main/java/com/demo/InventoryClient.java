package com.demo;

import org.springframework.cloud.openfeign.FeignClient;
import org.springframework.web.bind.annotation.GetMapping;

@FeignClient(name = "inventory", path = "/inventory")
public interface InventoryClient {
    @GetMapping("/items/{id}")
    InventoryItem lookup(String id);
}
