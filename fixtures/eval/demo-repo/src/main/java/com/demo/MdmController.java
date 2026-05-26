package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/mdm")
public class MdmController {
    private MdmService service;

    /** Query MDM by criteria. */
    @PostMapping("/query")
    public ApiResult queryMdm(@RequestBody MdmQueryRequest request) {
        return service.query(request);
    }
}

