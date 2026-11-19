// Unpacked arrays as subroutine arguments (IEEE 1800-2017 13.3, 7.6): an input is a copy
// (writes to it stay inside), outputs are copied back, elements pair from the left (also
// between arrays of opposite directions); ANSI and non-ANSI ports, automatic and static.
module t;
  integer errors = 0;
  int a[4];
  logic [7:0] x[0:2], y[0:2];
  byte z[1:3];
  function automatic int sum(input int v[4]);
    sum = 0;
    foreach (v[i]) sum += v[i];
    v[0] = 99;
  endfunction
  task automatic rev(input logic [7:0] i[0:2], output logic [7:0] o[0:2]);
    for (int k = 0; k < 3; k++) o[k] = i[2 - k];
  endtask
  task fill;
    output byte o[3:1];
    begin
      o[1] = 1;
      o[2] = 2;
      o[3] = 3;
    end
  endtask
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    foreach (a[i]) a[i] = i + 1;
    x[0] = 8'h10;
    x[1] = 8'h20;
    x[2] = 8'h30;
    check(`__LINE__, sum(a), 10);
    check(`__LINE__, a[0], 1);
    rev(x, y);
    check(`__LINE__, {y[0], y[1], y[2]}, 24'h302010);
    // o[3:1] is copied left to right into z[1:3]: z[1] = o[3]
    fill(z);
    check(`__LINE__, {z[1], z[2], z[3]}, 24'h030201);
    begin : blk
      // a block variable with a descending range pairs with o[3:1] index by index
      byte w[3:1];
      fill(w);
      check(`__LINE__, {w[3], w[2], w[1]}, 24'h030201);
    end
    if (errors == 0) $display("PASSED");
  end
endmodule
