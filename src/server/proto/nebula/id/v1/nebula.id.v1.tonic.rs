// @generated
/// Generated client implementations.
pub mod nebula_id_service_client {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use sdforge::tonic::codegen::*;
    use sdforge::tonic::codegen::http::Uri;
    #[derive(Debug, Clone)]
    pub struct NebulaIdServiceClient<T> {
        inner: sdforge::tonic::client::Grpc<T>,
    }
    impl NebulaIdServiceClient<sdforge::tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, sdforge::tonic::transport::Error>
        where
            D: TryInto<sdforge::tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = sdforge::tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> NebulaIdServiceClient<T>
    where
        T: sdforge::tonic::client::GrpcService<sdforge::tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = sdforge::tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = sdforge::tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> NebulaIdServiceClient<InterceptedService<T, F>>
        where
            F: sdforge::tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: sdforge::tonic::codegen::Service<
                http::Request<sdforge::tonic::body::Body>,
                Response = http::Response<
                    <T as sdforge::tonic::client::GrpcService<sdforge::tonic::body::Body>>::ResponseBody,
                >,
            >,
            <T as sdforge::tonic::codegen::Service<
                http::Request<sdforge::tonic::body::Body>,
            >>::Error: Into<StdError> + std::marker::Send + std::marker::Sync,
        {
            NebulaIdServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        pub async fn generate(
            &mut self,
            request: impl sdforge::tonic::IntoRequest<super::GenerateRequest>,
        ) -> std::result::Result<
            sdforge::tonic::Response<super::GenerateResponse>,
            sdforge::tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    sdforge::tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nebula.id.v1.NebulaIdService/Generate",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("nebula.id.v1.NebulaIdService", "Generate"));
            self.inner.unary(req, path, codec).await
        }
        pub async fn batch_generate(
            &mut self,
            request: impl sdforge::tonic::IntoRequest<super::BatchGenerateRequest>,
        ) -> std::result::Result<
            sdforge::tonic::Response<super::BatchGenerateResponse>,
            sdforge::tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    sdforge::tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nebula.id.v1.NebulaIdService/BatchGenerate",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("nebula.id.v1.NebulaIdService", "BatchGenerate"),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn parse(
            &mut self,
            request: impl sdforge::tonic::IntoRequest<super::ParseRequest>,
        ) -> std::result::Result<sdforge::tonic::Response<super::ParseResponse>, sdforge::tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    sdforge::tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nebula.id.v1.NebulaIdService/Parse",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("nebula.id.v1.NebulaIdService", "Parse"));
            self.inner.unary(req, path, codec).await
        }
        pub async fn health_check(
            &mut self,
            request: impl sdforge::tonic::IntoRequest<super::HealthCheckRequest>,
        ) -> std::result::Result<
            sdforge::tonic::Response<super::HealthCheckResponse>,
            sdforge::tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    sdforge::tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nebula.id.v1.NebulaIdService/HealthCheck",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("nebula.id.v1.NebulaIdService", "HealthCheck"));
            self.inner.unary(req, path, codec).await
        }
        pub async fn batch_generate_stream(
            &mut self,
            request: impl sdforge::tonic::IntoStreamingRequest<
                Message = super::BatchGenerateStreamRequest,
            >,
        ) -> std::result::Result<
            sdforge::tonic::Response<sdforge::tonic::codec::Streaming<super::BatchGenerateStreamResponse>>,
            sdforge::tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    sdforge::tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nebula.id.v1.NebulaIdService/BatchGenerateStream",
            );
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nebula.id.v1.NebulaIdService",
                        "BatchGenerateStream",
                    ),
                );
            self.inner.streaming(req, path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod nebula_id_service_server {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use sdforge::tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with NebulaIdServiceServer.
    #[async_trait]
    pub trait NebulaIdService: std::marker::Send + std::marker::Sync + 'static {
        async fn generate(
            &self,
            request: sdforge::tonic::Request<super::GenerateRequest>,
        ) -> std::result::Result<
            sdforge::tonic::Response<super::GenerateResponse>,
            sdforge::tonic::Status,
        >;
        async fn batch_generate(
            &self,
            request: sdforge::tonic::Request<super::BatchGenerateRequest>,
        ) -> std::result::Result<
            sdforge::tonic::Response<super::BatchGenerateResponse>,
            sdforge::tonic::Status,
        >;
        async fn parse(
            &self,
            request: sdforge::tonic::Request<super::ParseRequest>,
        ) -> std::result::Result<sdforge::tonic::Response<super::ParseResponse>, sdforge::tonic::Status>;
        async fn health_check(
            &self,
            request: sdforge::tonic::Request<super::HealthCheckRequest>,
        ) -> std::result::Result<
            sdforge::tonic::Response<super::HealthCheckResponse>,
            sdforge::tonic::Status,
        >;
        /// Server streaming response type for the BatchGenerateStream method.
        type BatchGenerateStreamStream: sdforge::tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::BatchGenerateStreamResponse,
                    sdforge::tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn batch_generate_stream(
            &self,
            request: sdforge::tonic::Request<sdforge::tonic::Streaming<super::BatchGenerateStreamRequest>>,
        ) -> std::result::Result<
            sdforge::tonic::Response<Self::BatchGenerateStreamStream>,
            sdforge::tonic::Status,
        >;
    }
    #[derive(Debug)]
    pub struct NebulaIdServiceServer<T> {
        inner: Arc<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    impl<T> NebulaIdServiceServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
                max_decoding_message_size: None,
                max_encoding_message_size: None,
            }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> InterceptedService<Self, F>
        where
            F: sdforge::tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
        /// Enable decompressing requests with the given encoding.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.accept_compression_encodings.enable(encoding);
            self
        }
        /// Compress responses with the given encoding, if the client supports it.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.send_compression_encodings.enable(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.max_decoding_message_size = Some(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.max_encoding_message_size = Some(limit);
            self
        }
    }
    impl<T, B> sdforge::tonic::codegen::Service<http::Request<B>> for NebulaIdServiceServer<T>
    where
        T: NebulaIdService,
        B: Body + std::marker::Send + 'static,
        B::Error: Into<StdError> + std::marker::Send + 'static,
    {
        type Response = http::Response<sdforge::tonic::body::Body>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            match req.uri().path() {
                "/nebula.id.v1.NebulaIdService/Generate" => {
                    #[allow(non_camel_case_types)]
                    struct GenerateSvc<T: NebulaIdService>(pub Arc<T>);
                    impl<
                        T: NebulaIdService,
                    > sdforge::tonic::server::UnaryService<super::GenerateRequest>
                    for GenerateSvc<T> {
                        type Response = super::GenerateResponse;
                        type Future = BoxFuture<
                            sdforge::tonic::Response<Self::Response>,
                            sdforge::tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: sdforge::tonic::Request<super::GenerateRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as NebulaIdService>::generate(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GenerateSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = sdforge::tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nebula.id.v1.NebulaIdService/BatchGenerate" => {
                    #[allow(non_camel_case_types)]
                    struct BatchGenerateSvc<T: NebulaIdService>(pub Arc<T>);
                    impl<
                        T: NebulaIdService,
                    > sdforge::tonic::server::UnaryService<super::BatchGenerateRequest>
                    for BatchGenerateSvc<T> {
                        type Response = super::BatchGenerateResponse;
                        type Future = BoxFuture<
                            sdforge::tonic::Response<Self::Response>,
                            sdforge::tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: sdforge::tonic::Request<super::BatchGenerateRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as NebulaIdService>::batch_generate(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = BatchGenerateSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = sdforge::tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nebula.id.v1.NebulaIdService/Parse" => {
                    #[allow(non_camel_case_types)]
                    struct ParseSvc<T: NebulaIdService>(pub Arc<T>);
                    impl<
                        T: NebulaIdService,
                    > sdforge::tonic::server::UnaryService<super::ParseRequest> for ParseSvc<T> {
                        type Response = super::ParseResponse;
                        type Future = BoxFuture<
                            sdforge::tonic::Response<Self::Response>,
                            sdforge::tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: sdforge::tonic::Request<super::ParseRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as NebulaIdService>::parse(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ParseSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = sdforge::tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nebula.id.v1.NebulaIdService/HealthCheck" => {
                    #[allow(non_camel_case_types)]
                    struct HealthCheckSvc<T: NebulaIdService>(pub Arc<T>);
                    impl<
                        T: NebulaIdService,
                    > sdforge::tonic::server::UnaryService<super::HealthCheckRequest>
                    for HealthCheckSvc<T> {
                        type Response = super::HealthCheckResponse;
                        type Future = BoxFuture<
                            sdforge::tonic::Response<Self::Response>,
                            sdforge::tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: sdforge::tonic::Request<super::HealthCheckRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as NebulaIdService>::health_check(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = HealthCheckSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = sdforge::tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nebula.id.v1.NebulaIdService/BatchGenerateStream" => {
                    #[allow(non_camel_case_types)]
                    struct BatchGenerateStreamSvc<T: NebulaIdService>(pub Arc<T>);
                    impl<
                        T: NebulaIdService,
                    > sdforge::tonic::server::StreamingService<super::BatchGenerateStreamRequest>
                    for BatchGenerateStreamSvc<T> {
                        type Response = super::BatchGenerateStreamResponse;
                        type ResponseStream = T::BatchGenerateStreamStream;
                        type Future = BoxFuture<
                            sdforge::tonic::Response<Self::ResponseStream>,
                            sdforge::tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: sdforge::tonic::Request<
                                sdforge::tonic::Streaming<super::BatchGenerateStreamRequest>,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as NebulaIdService>::batch_generate_stream(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = BatchGenerateStreamSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = sdforge::tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => {
                    Box::pin(async move {
                        let mut response = http::Response::new(
                            sdforge::tonic::body::Body::default(),
                        );
                        let headers = response.headers_mut();
                        headers
                            .insert(
                                sdforge::tonic::Status::GRPC_STATUS,
                                (sdforge::tonic::Code::Unimplemented as i32).into(),
                            );
                        headers
                            .insert(
                                http::header::CONTENT_TYPE,
                                sdforge::tonic::metadata::GRPC_CONTENT_TYPE,
                            );
                        Ok(response)
                    })
                }
            }
        }
    }
    impl<T> Clone for NebulaIdServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
                max_decoding_message_size: self.max_decoding_message_size,
                max_encoding_message_size: self.max_encoding_message_size,
            }
        }
    }
    /// Generated gRPC service name
    pub const SERVICE_NAME: &str = "nebula.id.v1.NebulaIdService";
    impl<T> sdforge::tonic::server::NamedService for NebulaIdServiceServer<T> {
        const NAME: &'static str = SERVICE_NAME;
    }
}
