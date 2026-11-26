// a vector port connected to a vector with several packed dimensions: the port's part
// selects are those of its own type
module e #(parameter n = 2, k = 4) (input [n-1:0] i, output logic [n*k-1:0] o);
  always_comb
    for (integer j = 0; j < n; j++)
      o[k*j+:k] = {k{i[j]}};
endmodule
module top;
  logic [1:0] i;
  logic [1:0][3:0] o;
  e #(2, 4) u(.i(i), .o(o));
  initial begin
    i = 2'b10;
    #1 if (o === 8'b1111_0000 && o[1] === 4'b1111) $display("PASSED");
    else $display("FAILED %b", o);
  end
endmodule
