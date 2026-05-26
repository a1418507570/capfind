package com.acme.customer.api;

import jakarta.ws.rs.GET;
import jakarta.ws.rs.Path;

@Path("/enterprise/customers")
public class CustomerResource {
    /**
     * Load customer profile through the JAX-RS resource layer.
     */
    @GET
    @Path("/{customerId}/profile")
    public CustomerProfile loadProfile(String customerId) {
        return null;
    }
}
