// expected
// Constant functions with `disable` of their named blocks (a loop's body and the whole
// body) and a $display, which constant evaluation ignores.
module top;
  function [31:0] pow2(input [5:0] x);
    begin : body
      pow2 = 1;
      forever begin : loop
        if (x == 0) disable body;
        pow2 = 2 * pow2;
        x = x - 1;
        disable loop;
        pow2 = 0;
      end
    end
  endfunction

  function integer first_set(input [15:0] v);
    integer i;
    begin : search
      first_set = -1;
      for (i = 0; i < 16; i = i + 1)
        if (v[i]) begin
          first_set = i;
          $display("found %0d", i);
          disable search;
        end
    end
  endfunction

  localparam P = pow2(10);
  localparam F = first_set(16'h0120);
  reg [first_set(16'h0008):0] r;

  initial begin
    $display("%0d %0d %0d %0d", P, F, $bits(r), first_set(16'h8000));
  end
endmodule
