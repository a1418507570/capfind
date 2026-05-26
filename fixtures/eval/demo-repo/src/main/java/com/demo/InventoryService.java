package com.demo;

import org.springframework.stereotype.Service;

@Service
public class InventoryService {
    private InventoryClient inventoryClient;

    public InventoryItem loadInventory(String id) {
        return inventoryClient.lookup(id);
    }
}
