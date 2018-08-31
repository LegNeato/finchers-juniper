use finchers::endpoint::{Context, Endpoint, EndpointResult};
use finchers::error;
use finchers::error::Error;

use std::pin::PinMut;

use futures::future::{Future, TryFuture};
use futures::task;
use futures::task::Poll;
use futures::try_ready;
use pin_utils::unsafe_pinned;

use juniper::{GraphQLType, RootNode};

use tokio::prelude::Async as Async01;
use tokio_threadpool::blocking;

use crate::maybe_done::MaybeDone;
use crate::request::{GraphQLResponse, RequestEndpoint, RequestFuture};

#[allow(missing_docs)]
pub struct Execute<E, QueryT, MutationT, CtxT>
where
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    pub(super) root_node: RootNode<'static, QueryT, MutationT>,
    pub(super) context_endpoint: E,
    pub(super) request_endpoint: RequestEndpoint,
}

impl<'a, E, QueryT, MutationT, CtxT> Endpoint<'a> for Execute<E, QueryT, MutationT, CtxT>
where
    E: Endpoint<'a, Output = (CtxT,)>,
    CtxT: 'a,
    QueryT: GraphQLType<Context = CtxT> + 'a,
    MutationT: GraphQLType<Context = CtxT> + 'a,
{
    type Output = (GraphQLResponse,);
    type Future = ExecuteFuture<'a, E::Future, QueryT, MutationT, CtxT>;

    fn apply(&'a self, cx: &mut Context) -> EndpointResult<Self::Future> {
        let request = self.request_endpoint.apply(cx)?;
        let context = self.context_endpoint.apply(cx)?;
        Ok(ExecuteFuture {
            request: MaybeDone::new(request),
            context: MaybeDone::new(context),
            root_node: &self.root_node,
        })
    }
}

pub struct ExecuteFuture<'a, F, QueryT, MutationT, CtxT>
where
    F: TryFuture<Ok = (CtxT,), Error = Error> + 'a,
    QueryT: GraphQLType<Context = CtxT> + 'a,
    MutationT: GraphQLType<Context = CtxT> + 'a,
{
    context: MaybeDone<F>,
    request: MaybeDone<RequestFuture<'a>>,
    root_node: &'a RootNode<'static, QueryT, MutationT>,
}

impl<'a, F, QueryT, MutationT, CtxT> ExecuteFuture<'a, F, QueryT, MutationT, CtxT>
where
    F: TryFuture<Ok = (CtxT,), Error = Error> + 'a,
    QueryT: GraphQLType<Context = CtxT> + 'a,
    MutationT: GraphQLType<Context = CtxT> + 'a,
{
    unsafe_pinned!(context: MaybeDone<F>);
    unsafe_pinned!(request: MaybeDone<RequestFuture<'a>>);
}

impl<'a, F, QueryT, MutationT, CtxT> Future for ExecuteFuture<'a, F, QueryT, MutationT, CtxT>
where
    F: TryFuture<Ok = (CtxT,), Error = Error> + 'a,
    QueryT: GraphQLType<Context = CtxT> + 'a,
    MutationT: GraphQLType<Context = CtxT> + 'a,
{
    type Output = Result<(GraphQLResponse,), Error>;

    fn poll(mut self: PinMut<Self>, cx: &mut task::Context) -> Poll<Self::Output> {
        try_ready!(self.request().poll_ready(cx));
        try_ready!(self.context().poll_ready(cx));
        match blocking(move || {
            let (request,) = self.request().take_ok().unwrap();
            let (context,) = self.context().take_ok().unwrap();
            request.execute(self.root_node, &context)
        }) {
            Ok(Async01::Ready(response)) => Poll::Ready(Ok((response,))),
            Ok(Async01::NotReady) => Poll::Pending,
            Err(err) => Poll::Ready(Err(error::fail(err))),
        }
    }
}
