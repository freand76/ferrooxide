// Members and methods of package variables reached by their scoped name (IEEE 1800-2017
// 26.3): struct members, enum methods, class properties, constants and methods; indexes and
// arguments are evaluated where the name is used.
package P;
  localparam N = 1;
  typedef enum { A, B, C, D } abcd_t;
  struct packed { logic [3:0] x; logic [3:0] y; } s = 8'h5a;
  abcd_t e = B;
  class K;
    localparam X = 4'b0101;
    int v = 7;
    function int f(int a); return a + v; endfunction
  endclass
  K k = new;
endpackage

module test;
  localparam N = 2;
  int fails = 0;
  int k3 = 3;
  initial begin
    if (P::s.x !== 4'h5 || P::s.y[N] !== 1'b0 || P::s.y[3] !== 1'b1) begin
      $display("FAILED struct %h %b", P::s.x, P::s.y); fails++;
    end
    if (P::e.next !== P::C || P::e.next(N) !== P::D || P::e.prev(N) !== P::D
        || P::e.next(k3) !== P::A || P::e.first !== P::A || P::e.num != 4 || P::e.name != "B") begin
      $display("FAILED enum %0d %0d %s", P::e.next(N), P::e.prev(N), P::e.name); fails++;
    end
    k3 = 6;
    if (P::e.next(k3) !== P::D || P::e.prev(k3) !== P::D) begin
      $display("FAILED enum run-time count"); fails++;
    end
    if (P::k.X[N] !== 1'b1 || P::k.v != 7 || P::k.f(N) != 9 || $bits(P::k.f(1)) != 32) begin
      $display("FAILED class %0d %0d", P::k.v, P::k.f(N)); fails++;
    end
    if (fails == 0) $display("PASSED");
  end
endmodule
