package com.demo;

import org.apache.dubbo.config.annotation.DubboReference;
import org.springframework.stereotype.Service;

@Service
public class CatalogGateway {
    @DubboReference
    private CatalogDubboService catalogDubboService;

    public CatalogItem loadCatalog(String id) {
        return catalogDubboService.getCatalog(id);
    }
}
