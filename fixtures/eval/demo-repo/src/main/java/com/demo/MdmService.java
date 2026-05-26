package com.demo;

import org.springframework.stereotype.Service;

@Service
public class MdmService {
    private MdmRepository repository;

    public ApiResult query(MdmQueryRequest request) {
        return repository.findById(request.id());
    }
}

