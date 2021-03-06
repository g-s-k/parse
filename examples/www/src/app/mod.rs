use yew::prelude::*;

use body::Body;
use footer::Footer;
use header::Header;

mod body;
mod footer;
mod header;

pub struct App;

impl Component for App {
    type Message = ();
    type Properties = ();

    fn create(_: Self::Properties, _: ComponentLink<Self>) -> Self {
        App
    }

    fn update(&mut self, _: Self::Message) -> ShouldRender {
        false
    }
}

impl Renderable<App> for App {
    fn view(&self) -> Html<Self> {
        html! {
            <>
                <Header: title="PARSLEY Scheme", subtitle="a Scheme implementation in Rust", />
                <Body: />
                <Footer: />
                </>
        }
    }
}
