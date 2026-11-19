// Assignments to package variables by their scoped name (IEEE 1800-2017 26.3: `P::v` is
// a variable like any other): whole, compound, increment, bit and part selects, array
// elements, struct members, strings, nonblocking; a process waiting on one wakes.
package P;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } s_t;
  int n;
  logic [7:0] v;
  int arr [4];
  s_t s;
  string str;
endpackage

module m;
  int seen = 0;
  always @(P::n) seen++;
  initial begin
    #1 P::n = 5;
    P::n += 2;
    P::n++;
    P::v = 8'h00;
    P::v[3] = 1;
    P::v[7:6] = 2'b11;
    P::arr[2] = 9;
    P::s.b = 4'h3;
    P::s.a = 4'hc;
    P::str = "hi";
    P::arr[1] <= 4;
    #1;
    if (P::n != 8 || P::v != 8'hc8 || P::arr[2] != 9 || P::arr[1] != 4 || P::s != 8'hc3 || P::str != "hi")
      $display("FAILED -- %0d %h %0d %0d %h %s", P::n, P::v, P::arr[2], P::arr[1], P::s, P::str);
    if (seen != 1) $display("FAILED -- seen %0d", seen);
    $display("PASSED");
  end
endmodule
