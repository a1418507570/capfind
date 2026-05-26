package com.acme.generated;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/generated")
public class GeneratedController {
    @GetMapping("/ignored")
    public String ignored() {
        return "ignored";
    }
}
