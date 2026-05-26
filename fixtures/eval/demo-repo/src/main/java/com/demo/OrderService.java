package com.demo;

import org.springframework.stereotype.Service;

@Service
public class OrderService {
    private OrderMapper orderMapper;

    public Order load(String id) {
        return orderMapper.selectById(id);
    }
}
