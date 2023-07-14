# harbinger

## CAVEATS
* to export a complete HAR from firefox: set `devtools.netmonitor.responseBodyLimit` to `0`

## TODO
* [x] proxy requests to some other backend (e.g. a python server for dynamic responses)
* [x] static file serving in the dump dir for entries not in the HAR
* [ ] handling of request params (currently `/foo?bar` and `/foo?baz` get assigned the same entry handler, despite possibly having different responses)
* [ ] user-friendly TUI walkthrough
* [ ] fix serviceworkers in firefox

## OPEN PROBLEMS
* [x] CSP applies before, during, and after requests (https://stackoverflow.com/questions/68390122/does-the-content-security-policy-csp-get-applied-before-requests-are-sent-to-s#comment120878673_68390122)
    * this means that `<img src=external.website/foo.jpg>` will be blocked by a strict `self` CSP, even if a service worker would rewrite it to a domain that passes
    * don't use CSP, rely on blackhole proxy to kill outgoing requests
* [x] frames whose src are an external resource are untouched by locally running serviceworkers
    * is there a localhost exception to this?
    * simply block 'em w/ a blackhole proxy
* [x] no general way to restrict outgoing network requests at the browser level. maybe at the OS level?
    * use chrome flags `--proxy-server=<harbinger blackhole server> --proxy-bypass-list=localhost`
