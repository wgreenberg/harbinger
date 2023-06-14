# harbinger

## CAVEATS
* export HAR from firefox: set `devtools.netmonitor.responseBodyLimit` to `0`

## TODO
* proxy requests to some other backend (e.g. a python server for dynamic responses)
* static file serving in the dump dir for entries not in the HAR
* handling of request params (currently `/foo?bar` and `/foo?baz` get assigned the same entry handler, despite possibly having different responses)

## PROBLEMS
* CSP applies before, during, and after requests (https://stackoverflow.com/questions/68390122/does-the-content-security-policy-csp-get-applied-before-requests-are-sent-to-s#comment120878673_68390122)
    * this means that `<img src=external.website/foo.jpg>` will be blocked by a strict `self` CSP, even if a service worker would rewrite it to a domain that passes
* frames whose src are an external resource are untouched by locally running serviceworkers
    * is there a localhost exception to this?
* no general way to restrict outgoing network requests at the browser level. maybe at the OS level?
* harbinger's serviceworker doesn't seem to work in firefox at all
