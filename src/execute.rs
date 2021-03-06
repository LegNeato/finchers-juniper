use finchers::endpoint::wrapper::Wrapper;
use finchers::endpoint::{Context, Endpoint, EndpointResult};
use finchers::error::Error;

use futures::future::Future;
use futures::ready;
use futures::task;
use futures::task::Poll;

use pin_utils::unsafe_pinned;
use std::pin::PinMut;

use juniper::{GraphQLType, RootNode};
use std::fmt;
use tokio::prelude::Async as Async01;
use tokio_threadpool::blocking;

use crate::maybe_done::MaybeDone;
use crate::request::{GraphQLResponse, RequestEndpoint};

/// Create a `Wrapper` for building a GraphQL endpoint using the specified `RootNode`.
///
/// The endpoint created by this wrapper will block the current thread
/// to execute the GraphQL query.
pub fn execute<QueryT, MutationT, CtxT>(
    root_node: RootNode<'static, QueryT, MutationT>,
) -> Execute<QueryT, MutationT, CtxT>
where
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    Execute {
        root_node,
        use_blocking: true,
    }
}

#[allow(missing_docs)]
pub struct Execute<QueryT, MutationT, CtxT>
where
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    root_node: RootNode<'static, QueryT, MutationT>,
    use_blocking: bool,
}

impl<QueryT, MutationT, CtxT> Execute<QueryT, MutationT, CtxT>
where
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    /// Sets whether to use the Tokio's blocking API when executing the GraphQL query.
    ///
    /// The default value is `true`.
    pub fn use_blocking(self, enabled: bool) -> Self {
        Execute {
            use_blocking: enabled,
            ..self
        }
    }
}

impl<QueryT, MutationT, CtxT> fmt::Debug for Execute<QueryT, MutationT, CtxT>
where
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Execute")
            .field("use_blocking", &self.use_blocking)
            .finish()
    }
}

impl<'a, E, QueryT, MutationT, CtxT> Wrapper<'a, E> for Execute<QueryT, MutationT, CtxT>
where
    E: Endpoint<'a, Output = (CtxT,)>,
    QueryT: GraphQLType<Context = CtxT> + 'a,
    MutationT: GraphQLType<Context = CtxT> + 'a,
    CtxT: 'a,
{
    type Output = (GraphQLResponse,);
    type Endpoint = ExecuteEndpoint<E, QueryT, MutationT, CtxT>;

    fn wrap(self, endpoint: E) -> Self::Endpoint {
        ExecuteEndpoint {
            context: endpoint,
            request: crate::request::request(),
            root_node: self.root_node,
            use_blocking: self.use_blocking,
        }
    }
}

pub struct ExecuteEndpoint<E, QueryT, MutationT, CtxT>
where
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    context: E,
    request: RequestEndpoint,
    root_node: RootNode<'static, QueryT, MutationT>,
    use_blocking: bool,
}

impl<E, QueryT, MutationT, CtxT> fmt::Debug for ExecuteEndpoint<E, QueryT, MutationT, CtxT>
where
    E: fmt::Debug,
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecuteEndpoint")
            .field("context", &self.context)
            .field("request", &self.request)
            .field("use_blocking", &self.use_blocking)
            .finish()
    }
}

impl<'a, E, QueryT, MutationT, CtxT> Endpoint<'a> for ExecuteEndpoint<E, QueryT, MutationT, CtxT>
where
    E: Endpoint<'a, Output = (CtxT,)>,
    QueryT: GraphQLType<Context = CtxT> + 'a,
    MutationT: GraphQLType<Context = CtxT> + 'a,
    CtxT: 'a,
{
    type Output = (GraphQLResponse,);
    type Future = ExecuteFuture<'a, E, QueryT, MutationT, CtxT>;

    fn apply(&'a self, cx: &mut Context<'_>) -> EndpointResult<Self::Future> {
        let context = self.context.apply(cx)?;
        let request = self.request.apply(cx)?;
        Ok(ExecuteFuture {
            context: MaybeDone::new(context),
            request: MaybeDone::new(request),
            endpoint: self,
        })
    }
}

#[derive(Debug)]
pub struct ExecuteFuture<'a, E, QueryT, MutationT, CtxT>
where
    E: Endpoint<'a, Output = (CtxT,)>,
    QueryT: GraphQLType<Context = CtxT> + 'a,
    MutationT: GraphQLType<Context = CtxT> + 'a,
    CtxT: 'a,
{
    context: MaybeDone<E::Future>,
    request: MaybeDone<<RequestEndpoint as Endpoint<'a>>::Future>,
    endpoint: &'a ExecuteEndpoint<E, QueryT, MutationT, CtxT>,
}

impl<'a, E, QueryT, MutationT, CtxT> ExecuteFuture<'a, E, QueryT, MutationT, CtxT>
where
    E: Endpoint<'a, Output = (CtxT,)>,
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
    CtxT: 'a,
{
    unsafe_pinned!(context: MaybeDone<E::Future>);
    unsafe_pinned!(request: MaybeDone<<RequestEndpoint as Endpoint<'a>>::Future>);

    fn execute(mut self: PinMut<'_, Self>) -> GraphQLResponse {
        let (context,) = self
            .context()
            .take_ok()
            .expect("The context has already taken");
        let (request,) = self
            .request()
            .take_ok()
            .expect("The request has already taken");
        request.execute(&self.endpoint.root_node, &context)
    }
}

impl<'a, E, QueryT, MutationT, CtxT> Future for ExecuteFuture<'a, E, QueryT, MutationT, CtxT>
where
    E: Endpoint<'a, Output = (CtxT,)>,
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
    CtxT: 'a,
{
    type Output = Result<(GraphQLResponse,), Error>;

    fn poll(mut self: PinMut<'_, Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        ready!(self.context().poll_ready(cx)?);
        ready!(self.request().poll_ready(cx)?);
        if self.endpoint.use_blocking {
            match blocking(move || self.execute()) {
                Ok(Async01::Ready(response)) => Poll::Ready(Ok((response,))),
                Ok(Async01::NotReady) => Poll::Pending,
                Err(err) => Poll::Ready(Err(finchers::error::fail(err))),
            }
        } else {
            let response = self.execute();
            Poll::Ready(Ok((response,)))
        }
    }
}
