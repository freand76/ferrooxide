// Parameters, typedefs and enums declared in a class (IEEE 1800-2017 8.5, 8.23): visible in
// its methods (before names of the enclosing scope) and in derived classes, and from outside
// as `C::P`, `obj.P` and `C::T`.
typedef logic [3:0] T;

module test;
  localparam W = 1;
  typedef enum { RED = 1 } color_e;

  class Base;
    localparam W = 16;
    parameter real SCALE = 2.5;
    typedef logic [W-1:0] T;
    typedef enum logic [1:0] { IDLE, BUSY, DONE } state_e;
    T data;
    state_e st;
    enum { RED = 7, GREEN } c;
    function int width();
      return $bits(data);
    endfunction
    function void step();
      st = st == IDLE ? BUSY : DONE;
    endfunction
  endclass

  class Derived extends Base;
    function int twice();
      return 2 * W + RED;
    endfunction
  endclass

  Base b;
  Derived d;
  Base::T word;
  T nib;
  integer fails = 0;

  task check(input integer got, input integer want, input integer line);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d, want %0d", line, got, want);
      fails++;
    end
  endtask

  initial begin
    b = new;
    d = new;
    check(b.width(), 16, `__LINE__);
    check(Base::W, 16, `__LINE__);
    check(b.W, 16, `__LINE__);
    check(W, 1, `__LINE__);
    check(RED, 1, `__LINE__);
    check(Base::RED, 7, `__LINE__);
    check(Base::GREEN, 8, `__LINE__);
    check(d.twice(), 39, `__LINE__);
    check($bits(word), 16, `__LINE__);
    check($bits(nib), 4, `__LINE__);
    if (b.SCALE != 2.5) begin $display("FAILED real parameter"); fails++; end
    b.st = Base::IDLE;
    b.step();
    check(b.st, Base::BUSY, `__LINE__);
    b.step();
    check(b.st, 2, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
