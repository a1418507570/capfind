package gateway

func register(router *gin.Engine, chiRouter chi.Router) {
    enterprise := router.Group("/enterprise/gateway")
    enterprise.POST("/orders/submit", submitOrder)
    chiRouter.Get("/enterprise/gateway/customers/{id}/profile", getCustomerProfile)
}
