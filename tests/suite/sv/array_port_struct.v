// an unpacked array port of a struct type connected to an array of the same type
typedef struct packed { logic [3:0] a; logic [3:0] b; } r_t;
module m #(parameter N = 2) (input r_t req_i [N], output logic [3:0] o [N]);
  for (genvar i = 0; i < N; i++) assign o[i] = req_i[i].a + req_i[i].b;
endmodule
module top;
  r_t req [3];
  logic [3:0] o [3];
  m #(3) u(.req_i(req), .o(o));
  initial begin
    req[0] = '{1, 2};
    req[1] = '{3, 4};
    req[2].a = 5;
    req[2].b = 6;
    #1 if (o[0] == 3 && o[1] == 7 && o[2] == 11) $display("PASSED");
    else $display("FAILED %0d %0d %0d", o[0], o[1], o[2]);
  end
endmodule
