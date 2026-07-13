<!-- Default slot exposing `thing: number` — a let: binding wrongly
     resolved against Foo would pick up this number type. -->
<slot thing={42} />
