package com.demo;

import org.apache.dubbo.config.annotation.DubboService;

@DubboService
public class CatalogDubboServiceImpl implements CatalogDubboService {
    public CatalogItem getCatalog(String id) {
        return null;
    }
}
