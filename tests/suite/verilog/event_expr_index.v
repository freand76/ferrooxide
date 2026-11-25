// expected
// An event control on an expression (`@(a[j])`, `@(posedge v[k])`) waits for a change from the
// expression's value when the wait starts: changing `j` just before must not fire it.
module top;
  reg [7:0] a [0:3];
  reg [3:0] v;
  integer j, k;
  initial begin
    j = 0;
    @(a[j]) $display("%0t first j=%0d", $time, j);
    j = 1;
    @(a[j]) $display("%0t second j=%0d a[1]=%0d", $time, j, a[1]);
  end
  initial begin
    v = 4'b0001;
    k = 1;
    #1 k = 0;
    @(posedge v[k]) $display("%0t posedge v[%0d]", $time, k);
  end
  initial #5 a[0] = 1;
  initial #8 a[1] = 1;
  initial #9 v = 4'b0000;
  initial #10 v = 4'b0001;
endmodule
