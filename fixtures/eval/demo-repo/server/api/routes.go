package api

func register(router *gin.Engine, chiRouter chi.Router) {
    v1 := router.Group("/v1")
    v1.GET("/go/mdm/query", queryMdm)
    chiRouter.Get("/chi/mdm/query", getMdm)
}

