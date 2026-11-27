// Every object has a random number generator of its own (IEEE 1800-2017 18.13.3-5,
// 18.14.1): `srandom`, `get_randstate` and `set_randstate` of any object, also called
// without `this.` in a method; `randomize()` draws from the object's generator, so two
// objects with the same seed get the same values.
class C;
  rand bit [31:0] x;
  function void seed(int s);
    srandom(s);
  endfunction
  function string state();
    return get_randstate();
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask

  C a, b;
  bit [31:0] v1, v2, v3;
  string s;

  initial begin
    a = new;
    b = new;
    a.srandom(7);
    b.srandom(7);
    check(`__LINE__, a.randomize(), 1);
    v1 = a.x;
    check(`__LINE__, b.randomize(), 1);
    check(`__LINE__, b.x, v1);
    // a value of another generator in between changes nothing
    v2 = $urandom;
    check(`__LINE__, a.randomize(), 1);
    check(`__LINE__, b.randomize(), 1);
    check(`__LINE__, a.x, b.x);
    s = a.get_randstate();
    void'(a.randomize());
    v1 = a.x;
    a.set_randstate(s);
    void'(a.randomize());
    check(`__LINE__, a.x, v1);
    b.seed(11);
    void'(b.randomize());
    v3 = b.x;
    a.srandom(11);
    void'(a.randomize());
    check(`__LINE__, a.x, v3);
    check(`__LINE__, a.state() == a.get_randstate(), 1);
    if (errors == 0) $display("PASSED");
  end
endmodule
