// a vector port connected (through another module) to a packed array of structs: the port's
// selects are those of its own type
module m #(parameter width_p = 4) (input clk_i, input [width_p-1:0] data_i, input [width_p-1:0] w_mask_i, output logic [width_p-1:0] q);
  always_ff @(posedge clk_i)
    for (integer i = 0; i < width_p; i = i + 1)
      if (w_mask_i[i])
        q[i] <= data_i[i];
endmodule
module w #(parameter width_p = 4) (input clk_i, input [width_p-1:0] data_i, input [width_p-1:0] w_mask_i, output [width_p-1:0] q);
  m #(.width_p(width_p)) synth (.clk_i, .data_i, .w_mask_i, .q);
endmodule
module top;
  typedef struct packed { logic [1:0] x, y; } s_t;
  logic clk;
  s_t [1:0] d, wm;
  logic [7:0] q;
  w #(.width_p(8)) u(.clk_i(clk), .data_i(d), .w_mask_i(wm), .q(q));
  initial begin
    q = 0;
    wm = 8'h0f;
    d = 8'h5a;
    clk = 0;
    #1 clk = 1;
    #1 if (q === 8'h0a && wm[0].y === 2'b11) $display("PASSED");
    else $display("FAILED %h", q);
  end
endmodule
